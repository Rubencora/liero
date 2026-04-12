//! `liero-relay` — NAT-traversal relay server for OpenLiero online matches.
//!
//! # Protocol
//!
//! Clients connect via TCP (native) or WebSocket (browser).
//! Both transports use the same text-based control protocol before entering
//! binary relay mode:
//!
//! ## Client → server
//! - `CREATE\n`                 — create a new room, receive its 6-letter code
//! - `JOIN:ABCDEF\n`            — join room ABCDEF
//!
//! ## Server → client
//! - `ROOM:ABCDEF\n`            — room created, you are the host
//! - `PAIRED\n`                 — a second player joined; now in relay mode
//! - `ERROR:reason\n`           — something went wrong
//!
//! ## Relay mode
//! Once both players are paired, every TCP/WS message from one is forwarded
//! verbatim to the other.  The connection ends when either peer disconnects.
//!
//! ## Room codes
//! 6 uppercase letters (A-Z), randomly generated, valid for 5 minutes or until
//! filled.
//!
//! # Usage
//! ```
//! cargo run --release -p liero-relay -- [--tcp-port 7777] [--ws-port 7778]
//! ```

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
    time::timeout,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

// ── Constants ─────────────────────────────────────────────────────────────────

const ROOM_TTL: Duration = Duration::from_secs(300);  // 5 minutes max wait for pair
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MSG: usize = 65536;

// ── Room registry ─────────────────────────────────────────────────────────────

/// Bidirectional pair info exchanged when two peers meet.
///
/// Each peer gets a `PairInfo` with a channel to send TO the other peer and
/// a channel to receive FROM the other peer.
struct PairInfo {
    to_peer:   mpsc::Sender<Vec<u8>>,
    from_peer: mpsc::Receiver<Vec<u8>>,
}

enum RoomMode {
    /// Standard relay mode: forward all bytes between peers.
    Relay { pair_tx: mpsc::Sender<PairInfo> },
    /// Signal-only mode: exchange public IPs then disconnect.
    Signal {
        host_tcp_addr: SocketAddr,
        host_udp_port: u16,
        notify_tx: mpsc::Sender<SocketAddr>,
    },
}

struct Room {
    mode: RoomMode,
    created: Instant,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

// ── Room code helpers ─────────────────────────────────────────────────────────

fn random_code() -> String {
    use std::time::UNIX_EPOCH;
    // Simple 64-bit LCG seeded from nanoseconds — no rand crate dependency.
    let seed = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let mut rng = seed as u64 ^ 0xDEAD_BEEF_1234_5678;
    let mut code = String::with_capacity(6);
    for _ in 0..6 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let c = b'A' + ((rng >> 33) as u8 % 26);
        code.push(c as char);
    }
    code
}

fn unique_code(rooms: &HashMap<String, Room>) -> String {
    for _ in 0..20 {
        let code = random_code();
        if !rooms.contains_key(&code) {
            return code;
        }
    }
    random_code() + "XY" // extremely unlikely fallback
}

fn evict_expired(rooms: &mut HashMap<String, Room>) {
    rooms.retain(|_, r| r.created.elapsed() < ROOM_TTL);
}

// ── Core relay logic ──────────────────────────────────────────────────────────

/// Drive one side of a paired relay: read from `rx` and write to `socket_tx`.
async fn relay_recv_to_socket<F, Fut>(
    mut rx: mpsc::Receiver<Vec<u8>>,
    send: F,
) where
    F: Fn(Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    while let Some(data) = rx.recv().await {
        if !send(data).await {
            break;
        }
    }
}

// ── TCP connection handler ─────────────────────────────────────────────────────

async fn handle_tcp(stream: TcpStream, rooms: Rooms, addr: SocketAddr) {
    if let Err(e) = tcp_lifecycle(stream, rooms, addr).await {
        eprintln!("[tcp {addr}] {e}");
    }
}

async fn tcp_lifecycle(mut stream: TcpStream, rooms: Rooms, addr: SocketAddr) -> Result<()> {
    // ── Handshake: read one line (up to 100 bytes) ─────────────────────────────
    let cmd = {
        let mut buf = Vec::with_capacity(64);
        let fut = async {
            let mut byte = [0u8; 1];
            loop {
                let n = stream.read(&mut byte).await?;
                if n == 0 || byte[0] == b'\n' { break; }
                if buf.len() < 100 { buf.push(byte[0]); }
            }
            Ok::<_, anyhow::Error>(String::from_utf8_lossy(&buf).trim().to_string())
        };
        timeout(HANDSHAKE_TIMEOUT, fut).await
            .map_err(|_| anyhow::anyhow!("handshake timeout"))?
            .map_err(|e| anyhow::anyhow!("read error: {e}"))?
    };

    if cmd == "CREATE" {
        tcp_create(stream, rooms, addr).await
    } else if let Some(port_str) = cmd.strip_prefix("SIGNAL:") {
        tcp_signal(stream, rooms, addr, port_str.trim()).await
    } else if let Some(code) = cmd.strip_prefix("JOIN:") {
        tcp_join(stream, rooms, addr, code.trim().to_uppercase()).await
    } else {
        stream.write_all(b"ERROR:unknown command\n").await?;
        Ok(())
    }
}

async fn tcp_create(mut stream: TcpStream, rooms: Rooms, addr: SocketAddr) -> Result<()> {
    let (pair_tx, mut pair_rx) = mpsc::channel::<PairInfo>(1);
    let code = {
        let mut guard = rooms.lock().await;
        evict_expired(&mut guard);
        let code = unique_code(&guard);
        guard.insert(code.clone(), Room { mode: RoomMode::Relay { pair_tx }, created: Instant::now() });
        code
    };
    eprintln!("[tcp {addr}] room {code} created");
    stream.write_all(format!("ROOM:{code}\n").as_bytes()).await?;

    // Wait for a joiner.
    let PairInfo { to_peer, from_peer } = timeout(ROOM_TTL, pair_rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("room {code} timed out waiting for joiner"))?
        .ok_or_else(|| anyhow::anyhow!("pair channel closed"))?;

    stream.write_all(b"PAIRED\n").await?;
    eprintln!("[tcp {addr}] room {code} paired");

    run_tcp_relay(stream, to_peer, from_peer, addr).await
}

async fn tcp_signal(
    mut stream: TcpStream,
    rooms: Rooms,
    addr: SocketAddr,
    port_str: &str,
) -> Result<()> {
    let host_udp_port: u16 = port_str.parse()
        .map_err(|_| anyhow::anyhow!("invalid port in SIGNAL command"))?;

    let (notify_tx, mut notify_rx) = mpsc::channel::<SocketAddr>(1);
    let code = {
        let mut guard = rooms.lock().await;
        evict_expired(&mut guard);
        let code = unique_code(&guard);
        guard.insert(
            code.clone(),
            Room {
                mode: RoomMode::Signal {
                    host_tcp_addr: addr,
                    host_udp_port,
                    notify_tx,
                },
                created: Instant::now(),
            },
        );
        code
    };

    eprintln!("[tcp {addr}] signal room {code} created on UDP port {host_udp_port}");
    stream.write_all(format!("ROOM:{code}\n").as_bytes()).await?;

    // Wait for a joiner.
    let joiner_addr = timeout(ROOM_TTL, notify_rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("signal room {code} timed out waiting for joiner"))?
        .ok_or_else(|| anyhow::anyhow!("notify channel closed"))?;

    eprintln!("[tcp {addr}] signal room {code} paired with {joiner_addr}");
    stream.write_all(format!("PEER:{}\n", joiner_addr.ip()).as_bytes()).await?;
    Ok(())
}

async fn tcp_join(
    mut stream: TcpStream,
    rooms: Rooms,
    addr: SocketAddr,
    code: String,
) -> Result<()> {
    let room = {
        let mut guard = rooms.lock().await;
        guard.remove(&code)
            .ok_or_else(|| anyhow::anyhow!("room {code} not found"))?
    };

    match room.mode {
        RoomMode::Relay { pair_tx } => {
            // Build our own pair and send host's pair info.
            let (host_tx, joiner_rx) = mpsc::channel::<Vec<u8>>(64); // host → joiner
            let (joiner_tx, host_rx) = mpsc::channel::<Vec<u8>>(64); // joiner → host

            // Send host its pair data: it receives from joiner (host_rx) and sends to joiner (host_tx).
            pair_tx.send(PairInfo { to_peer: host_tx, from_peer: host_rx }).await
                .map_err(|_| anyhow::anyhow!("host disconnected before JOIN"))?;

            eprintln!("[tcp {addr}] joined relay room {code}");
            stream.write_all(b"PAIRED\n").await?;

            // Joiner: receives from host (joiner_rx) and sends to host (joiner_tx).
            run_tcp_relay(stream, joiner_tx, joiner_rx, addr).await
        }
        RoomMode::Signal { host_tcp_addr, host_udp_port, notify_tx } => {
            // Signaling mode: send host IP:port to joiner, and joiner IP to host.
            eprintln!("[tcp {addr}] joined signal room {code}");
            stream.write_all(format!("PEER:{host_tcp_addr}:{host_udp_port}\n").as_bytes()).await?;

            // Notify the host about the joiner.
            let _ = notify_tx.send(addr).await;
            Ok(())
        }
    }
}

/// Bidirectional raw TCP relay after pairing.
async fn run_tcp_relay(
    stream: TcpStream,
    to_peer: mpsc::Sender<Vec<u8>>,
    from_peer: mpsc::Receiver<Vec<u8>>,
    addr: SocketAddr,
) -> Result<()> {
    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer));

    // Task: bytes from channel → this peer's socket.
    let wclone = Arc::clone(&writer);
    let write_task = tokio::spawn(relay_recv_to_socket(from_peer, move |data| {
        let w = Arc::clone(&wclone);
        async move {
            w.lock().await.write_all(&data).await.is_ok()
        }
    }));

    // Read bytes from this peer's socket → send to channel.
    let mut buf = vec![0u8; MAX_MSG];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to_peer.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        }
    }

    eprintln!("[tcp {addr}] disconnected");
    write_task.abort();
    Ok(())
}

// ── WebSocket connection handler ───────────────────────────────────────────────

async fn handle_ws(stream: TcpStream, rooms: Rooms, addr: SocketAddr) {
    if let Err(e) = ws_lifecycle(stream, rooms, addr).await {
        eprintln!("[ws {addr}] {e}");
    }
}

async fn ws_lifecycle(stream: TcpStream, rooms: Rooms, addr: SocketAddr) -> Result<()> {
    let ws = accept_async(stream).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();

    // ── Handshake: read the first WS text/binary message ──────────────────────
    let cmd = timeout(HANDSHAKE_TIMEOUT, ws_rx.next())
        .await
        .map_err(|_| anyhow::anyhow!("ws handshake timeout"))?
        .ok_or_else(|| anyhow::anyhow!("ws closed before handshake"))??;

    let cmd_str = match &cmd {
        Message::Text(t)   => t.as_str().trim().to_string(),
        Message::Binary(b) => String::from_utf8_lossy(b).trim().to_string(),
        _ => return Ok(()),
    };

    if cmd_str == "CREATE" {
        ws_create(ws_tx, ws_rx, rooms, addr).await
    } else if cmd_str.starts_with("SIGNAL:") {
        // WebSocket clients use CREATE, not SIGNAL (SIGNAL is TCP-only for native clients).
        ws_tx.send(Message::Text("ERROR:SIGNAL not supported over WebSocket".into())).await?;
        Ok(())
    } else if let Some(code) = cmd_str.strip_prefix("JOIN:") {
        ws_join(ws_tx, ws_rx, rooms, addr, code.trim().to_uppercase()).await
    } else {
        ws_tx.send(Message::Text("ERROR:unknown command".into())).await?;
        Ok(())
    }
}

async fn ws_create(
    mut ws_tx: impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Unpin + 'static,
    ws_rx: impl StreamExt<Item = tokio_tungstenite::tungstenite::Result<Message>> + Send + Unpin + 'static,
    rooms: Rooms,
    addr: SocketAddr,
) -> Result<()> {
    let (pair_tx, mut pair_rx) = mpsc::channel::<PairInfo>(1);
    let code = {
        let mut guard = rooms.lock().await;
        evict_expired(&mut guard);
        let code = unique_code(&guard);
        guard.insert(code.clone(), Room { mode: RoomMode::Relay { pair_tx }, created: Instant::now() });
        code
    };
    eprintln!("[ws {addr}] room {code} created");
    ws_tx.send(Message::Text(format!("ROOM:{code}").into())).await?;

    let PairInfo { to_peer, from_peer } = timeout(ROOM_TTL, pair_rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("room {code} timed out"))?
        .ok_or_else(|| anyhow::anyhow!("pair channel closed"))?;

    ws_tx.send(Message::Text("PAIRED".into())).await?;
    eprintln!("[ws {addr}] room {code} paired");

    run_ws_relay(ws_tx, ws_rx, to_peer, from_peer, addr).await
}

async fn ws_join(
    mut ws_tx: impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Unpin + 'static,
    ws_rx: impl StreamExt<Item = tokio_tungstenite::tungstenite::Result<Message>> + Send + Unpin + 'static,
    rooms: Rooms,
    addr: SocketAddr,
    code: String,
) -> Result<()> {
    let room = {
        let mut guard = rooms.lock().await;
        guard.remove(&code)
            .ok_or_else(|| anyhow::anyhow!("room {code} not found"))?
    };

    match room.mode {
        RoomMode::Relay { pair_tx } => {
            let (host_tx, joiner_rx) = mpsc::channel::<Vec<u8>>(64);
            let (joiner_tx, host_rx)  = mpsc::channel::<Vec<u8>>(64);

            pair_tx.send(PairInfo { to_peer: host_tx, from_peer: host_rx }).await
                .map_err(|_| anyhow::anyhow!("host disconnected before JOIN"))?;

            eprintln!("[ws {addr}] joined relay room {code}");
            ws_tx.send(Message::Text("PAIRED".into())).await?;

            run_ws_relay(ws_tx, ws_rx, joiner_tx, joiner_rx, addr).await
        }
        RoomMode::Signal { .. } => {
            ws_tx.send(Message::Text("ERROR:cannot join signal room over WebSocket".into())).await?;
            Ok(())
        }
    }
}

async fn run_ws_relay(
    ws_tx: impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Unpin + 'static,
    mut ws_rx: impl StreamExt<Item = tokio_tungstenite::tungstenite::Result<Message>> + Send + Unpin + 'static,
    to_peer: mpsc::Sender<Vec<u8>>,
    from_peer: mpsc::Receiver<Vec<u8>>,
    addr: SocketAddr,
) -> Result<()> {
    let ws_tx = Arc::new(Mutex::new(ws_tx));

    // Task: bytes from channel → this WS peer.
    let wclone = Arc::clone(&ws_tx);
    let write_task = tokio::spawn(relay_recv_to_socket(from_peer, move |data| {
        let w = Arc::clone(&wclone);
        async move {
            w.lock().await.send(Message::Binary(data.into())).await.is_ok()
        }
    }));

    // Read WS messages → forward to channel.
    while let Some(msg) = ws_rx.next().await {
        let data = match msg? {
            Message::Binary(b) => b.into(),
            Message::Text(t)   => t.as_bytes().to_vec(),
            Message::Close(_)  => break,
            _                  => continue,
        };
        if to_peer.send(data).await.is_err() {
            break;
        }
    }

    eprintln!("[ws {addr}] disconnected");
    write_task.abort();
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let mut tcp_port: u16 = 7777;
    let mut ws_port:  u16 = 7778;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tcp-port" => tcp_port = args.next().and_then(|s| s.parse().ok()).unwrap_or(tcp_port),
            "--ws-port"  => ws_port  = args.next().and_then(|s| s.parse().ok()).unwrap_or(ws_port),
            _ => {}
        }
    }

    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    let tcp_addr: SocketAddr = format!("0.0.0.0:{tcp_port}").parse()?;
    let ws_addr:  SocketAddr = format!("0.0.0.0:{ws_port}").parse()?;

    let tcp_listener = TcpListener::bind(tcp_addr).await?;
    let ws_listener  = TcpListener::bind(ws_addr).await?;

    eprintln!("liero-relay started");
    eprintln!("  TCP : 0.0.0.0:{tcp_port}  (native clients)");
    eprintln!("  WS  : 0.0.0.0:{ws_port}   (browser / WASM clients)");

    let rooms_tcp = Arc::clone(&rooms);
    let rooms_ws  = Arc::clone(&rooms);

    let tcp_task = tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((s, a)) => { let r = Arc::clone(&rooms_tcp); tokio::spawn(handle_tcp(s, r, a)); }
                Err(e)     => eprintln!("[tcp] accept error: {e}"),
            }
        }
    });

    let ws_task = tokio::spawn(async move {
        loop {
            match ws_listener.accept().await {
                Ok((s, a)) => { let r = Arc::clone(&rooms_ws); tokio::spawn(handle_ws(s, r, a)); }
                Err(e)     => eprintln!("[ws] accept error: {e}"),
            }
        }
    });

    tokio::try_join!(tcp_task, ws_task)?;
    Ok(())
}
