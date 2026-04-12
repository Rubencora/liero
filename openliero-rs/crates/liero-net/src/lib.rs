//! `liero-net` — rollback netcode over UDP.
//!
//! # Architecture
//! ```
//! [local host]                          [remote peer]
//!   game.step(local+predicted inputs)
//!   send NetPacket (local input + FEC)  ──────────────►
//!                                       recv NetPacket
//!                                       if rollback needed: restore snapshot, re-simulate
//! ◄──────────────  send NetPacket (remote input + FEC)
//! recv NetPacket
//! if rollback needed: restore snapshot, re-simulate
//! ```
//!
//! # Rollback window
//! Up to 8 frames of prediction.  If the remote peer is > 8 frames behind, we
//! stall (wait) instead of accumulating more prediction error.
//!
//! # Packet format (25 bytes, no external deps)
//! ```text
//! [0..4]   local_frame   : u32 LE  — frame counter of the included input
//! [4..5]   player_idx    : u8      — 0 or 1 (tells peer which input slot to fill)
//! [5..17]  inputs        : [u32;3] LE — inputs for frames [frame-2, frame-1, frame]
//! [17..25] checksum      : u64 LE  — game checksum at local_frame (for desync detection)
//! ```
//!
//! # Snapshot ring
//! 16 slots.  Saved once per frame before `step()`.  On rollback: restore the
//! slot matching the last confirmed frame, then re-simulate forward.

use std::net::{SocketAddr, UdpSocket};
use liero_data::Tc;
use liero_sim::game::Game;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum prediction depth in frames.  If remote is further behind, we stall.
pub const MAX_ROLLBACK: usize = 8;

/// Number of snapshot slots in the ring buffer (must be power of 2, ≥ MAX_ROLLBACK).
const SNAP_RING: usize = 16;

/// Fixed packet size in bytes.
const PACKET_LEN: usize = 25;

/// Input history ring size.
const INPUT_RING: usize = 256;

// ── Packet ────────────────────────────────────────────────────────────────────

/// A compact fixed-size packet sent/received over UDP.
#[derive(Debug, Clone, Copy)]
pub struct NetPacket {
    /// Frame number this packet's primary input corresponds to.
    pub frame:      u32,
    /// Which player index (0 or 1) is sending this packet.
    pub player_idx: u8,
    /// Last 3 inputs: [frame-2, frame-1, frame] (FEC — redundancy against packet loss).
    pub inputs:     [u32; 3],
    /// Checksum of sender's game state at `frame` (for desync detection).
    pub checksum:   u64,
}

impl NetPacket {
    pub fn encode(&self) -> [u8; PACKET_LEN] {
        let mut buf = [0u8; PACKET_LEN];
        buf[0..4].copy_from_slice(&self.frame.to_le_bytes());
        buf[4] = self.player_idx;
        buf[5..9].copy_from_slice(&self.inputs[0].to_le_bytes());
        buf[9..13].copy_from_slice(&self.inputs[1].to_le_bytes());
        buf[13..17].copy_from_slice(&self.inputs[2].to_le_bytes());
        buf[17..25].copy_from_slice(&self.checksum.to_le_bytes());
        buf
    }

    pub fn decode(buf: &[u8; PACKET_LEN]) -> Self {
        Self {
            frame:      u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            player_idx: buf[4],
            inputs: [
                u32::from_le_bytes(buf[5..9].try_into().unwrap()),
                u32::from_le_bytes(buf[9..13].try_into().unwrap()),
                u32::from_le_bytes(buf[13..17].try_into().unwrap()),
            ],
            checksum: u64::from_le_bytes(buf[17..25].try_into().unwrap()),
        }
    }
}

// ── Rollback session ──────────────────────────────────────────────────────────

/// Compute a stable FNV-1a hash of the critical TC data fields.
///
/// Used to detect TC mismatches between peers at the start of an online session.
/// Hashes: palette bytes, weapon count + names, nobject count, sobject count.
pub fn tc_hash_of(tc: &Tc) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME:  u64 = 0x0000_0100_0000_01b3;

    let mut h = FNV_OFFSET;
    let fnv_byte = |h: &mut u64, b: u8| {
        *h ^= b as u64;
        *h = h.wrapping_mul(FNV_PRIME);
    };

    // Palette
    for &b in &tc.palette { fnv_byte(&mut h, b); }

    // Weapon count + names (order-sensitive)
    fnv_byte(&mut h, tc.weapons.len() as u8);
    for w in &tc.weapons {
        for &b in w.name.as_bytes() { fnv_byte(&mut h, b); }
        fnv_byte(&mut h, 0);
    }

    // Nobject + sobject counts
    fnv_byte(&mut h, tc.nobjects.len() as u8);
    fnv_byte(&mut h, tc.sobjects.len() as u8);

    h
}

/// Rollback netcode session for a peer-to-peer match.
///
/// Drives `Game::step()` with local + predicted remote inputs.
/// When a remote packet arrives with real input, rolls back if needed,
/// re-simulates, and continues.
///
/// `local_player` = 0 → this host controls worm 0; remote controls worm 1.
/// `local_player` = 1 → this host controls worm 1; remote controls worm 0.
pub struct RollbackSession {
    socket:      UdpSocket,
    remote_addr: Option<SocketAddr>,

    local_player:  usize,
    remote_player: usize,

    /// Current local frame counter.
    pub local_frame: u32,
    /// Last frame for which we have a confirmed remote input.
    confirmed_frame: u32,

    /// Local input history ring (index = frame % INPUT_RING).
    local_inputs:  Box<[u32; INPUT_RING]>,
    /// Remote input history ring.  Entries beyond `confirmed_frame` are predictions.
    remote_inputs: Box<[u32; INPUT_RING]>,

    /// Snapshot ring buffer (slot = frame % SNAP_RING).
    snapshots: Vec<Option<GameSnapshot>>,

    /// FNV-1a hash of the local TC (set via `set_tc_hash`).
    pub tc_hash: u64,

    /// Set when a checksum mismatch is detected.
    pub desync_detected: bool,
    /// Frame at which the last desync was detected.
    pub desync_frame: u32,
    /// Human-readable reason for the desync (set when `desync_detected` is true).
    pub desync_message: String,
}

impl RollbackSession {
    /// Create a hosting session bound to `local_addr`.
    pub fn host(local_addr: SocketAddr, local_player: usize) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind(local_addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self::new(socket, None, local_player))
    }

    /// Create a client session connecting to `remote_addr`.
    pub fn connect(local_addr: SocketAddr, remote_addr: SocketAddr, local_player: usize) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind(local_addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self::new(socket, Some(remote_addr), local_player))
    }

    fn new(socket: UdpSocket, remote_addr: Option<SocketAddr>, local_player: usize) -> Self {
        let snapshots = (0..SNAP_RING).map(|_| None).collect();
        Self {
            socket,
            remote_addr,
            local_player,
            remote_player: 1 - local_player,
            local_frame: 0,
            confirmed_frame: 0,
            local_inputs:  Box::new([0u32; INPUT_RING]),
            remote_inputs: Box::new([0u32; INPUT_RING]),
            snapshots,
            tc_hash: 0,
            desync_detected: false,
            desync_frame: 0,
            desync_message: String::new(),
        }
    }

    /// Set the TC hash for this session.
    ///
    /// Call this immediately after creating the session, passing `tc_hash_of(&tc)`.
    /// Used to provide a clearer desync message when TC mismatches are detected early.
    pub fn set_tc_hash(&mut self, hash: u64) {
        self.tc_hash = hash;
    }

    /// Learn the remote address (useful when host waits for first packet from client).
    pub fn set_remote(&mut self, addr: SocketAddr) {
        self.remote_addr = Some(addr);
    }

    /// Advance the simulation by one frame with rollback support.
    ///
    /// 1. Saves snapshot for the current frame.
    /// 2. Drains incoming packets (may trigger rollback + re-sim).
    /// 3. Steps the sim with local input + best-available remote input.
    /// 4. Sends our input to the peer.
    ///
    /// Returns `true` if the step was executed, `false` if we stalled.
    pub fn step(&mut self, game: &mut Game, local_raw: u32) -> bool {
        if self.should_stall() {
            // Only drain packets while stalled — don't advance local_frame.
            self.drain_packets(game);
            return false;
        }

        // ── 1. Save snapshot ──────────────────────────────────────────────────
        let snap_slot = self.local_frame as usize % SNAP_RING;
        self.snapshots[snap_slot] = Some(game.save_snapshot());

        // ── 2. Drain incoming packets (may rollback + re-sim) ─────────────────
        self.drain_packets(game);

        // ── 3. Record local input + pick remote (predict) ─────────────────────
        let li = self.local_frame as usize % INPUT_RING;
        self.local_inputs[li] = local_raw;

        let pred_remote = self.remote_inputs[self.confirmed_frame as usize % INPUT_RING];
        let mut inputs = [0u32; 4];
        inputs[self.local_player]  = local_raw;
        inputs[self.remote_player] = pred_remote;

        game.step(&inputs);

        // ── 4. Send packet ────────────────────────────────────────────────────
        self.send_packet(game);

        self.local_frame += 1;
        true
    }

    /// Returns `true` if we should stall this frame (remote too far behind).
    pub fn should_stall(&self) -> bool {
        let ahead = self.local_frame.saturating_sub(self.confirmed_frame + 1);
        ahead as usize >= MAX_ROLLBACK
    }

    /// Number of frames of prediction ahead of confirmed remote state.
    pub fn prediction_frames(&self) -> u32 {
        self.local_frame.saturating_sub(self.confirmed_frame + 1)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn drain_packets(&mut self, game: &mut Game) {
        let mut buf = [0u8; PACKET_LEN];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if n != PACKET_LEN { continue; }
                    if self.remote_addr.is_none() {
                        self.remote_addr = Some(from);
                    }
                    let pkt = NetPacket::decode(buf[..PACKET_LEN].try_into().unwrap());
                    self.handle_packet(game, &pkt, from);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn handle_packet(&mut self, game: &mut Game, pkt: &NetPacket, from: SocketAddr) {
        if pkt.player_idx as usize != self.remote_player { return; }
        if self.remote_addr.is_none() {
            self.remote_addr = Some(from);
        }

        // Apply FEC inputs (up to 3 frames worth).
        let mut rollback_needed = false;
        let mut rollback_to = pkt.frame;

        for (i, &inp) in pkt.inputs.iter().enumerate() {
            let f = pkt.frame.saturating_sub(2 - i as u32);
            if f == 0 { continue; }

            let slot = f as usize % INPUT_RING;
            let prev = self.remote_inputs[slot];

            if f > self.confirmed_frame {
                self.remote_inputs[slot] = inp;
                self.confirmed_frame = f;
            } else if self.remote_inputs[slot] != inp {
                // Correction to a past frame — rollback needed.
                self.remote_inputs[slot] = inp;
                if f < rollback_to { rollback_to = f; }
                rollback_needed = true;
            }
            let _ = prev;
        }

        // Desync detection: compare checksums when the remote is exactly one step
        // behind us (the most common timing in a well-connected session).
        // At frames ≤ 5 any mismatch is almost certainly a TC mismatch.
        if !self.desync_detected
            && pkt.frame + 1 == self.local_frame
            && pkt.checksum != game.checksum()
        {
            self.desync_detected = true;
            self.desync_frame    = pkt.frame;
            self.desync_message  = if pkt.frame <= 5 {
                format!(
                    "TC mismatch at frame {} — ensure both players load the same TC \
                     (local hash: {:016x})",
                    pkt.frame, self.tc_hash
                )
            } else {
                format!("Game state desync at frame {}", pkt.frame)
            };
        }

        // Rollback + re-simulate if corrections arrived.
        if rollback_needed && rollback_to < self.local_frame {
            let age = self.local_frame - rollback_to;
            if age as usize > SNAP_RING { return; } // too old, ignore

            let restore_slot = rollback_to.saturating_sub(1) as usize % SNAP_RING;
            if let Some(snap) = self.snapshots[restore_slot].clone() {
                game.restore_snapshot(&snap);
                for f in rollback_to..self.local_frame {
                    let li = f as usize % INPUT_RING;
                    let ri = f as usize % INPUT_RING;
                    let mut inputs = [0u32; 4];
                    inputs[self.local_player]  = self.local_inputs[li];
                    inputs[self.remote_player] = self.remote_inputs[ri];
                    game.step(&inputs);
                    self.snapshots[f as usize % SNAP_RING] = Some(game.save_snapshot());
                }
            }
        }
    }

    fn send_packet(&self, game: &Game) {
        let Some(remote) = self.remote_addr else { return; };
        let f = self.local_frame;
        let fi = |off: u32| -> u32 {
            self.local_inputs[f.saturating_sub(off) as usize % INPUT_RING]
        };
        let pkt = NetPacket {
            frame:      f,
            player_idx: self.local_player as u8,
            inputs:     [fi(2), fi(1), fi(0)],
            checksum:   game.checksum(),
        };
        let _ = self.socket.send_to(&pkt.encode(), remote);
    }
}

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use liero_sim::game::GameSnapshot;

// ── Relay matchmaking ────────────────────────────────────────────────────────

/// Connect to the relay server, create a signaling room.
/// Returns the room code as soon as it's available.
/// Blocks until a peer joins (or TCP error / timeout).
///
/// Call from a background thread — do NOT call from the main game loop.
///
/// The callback `on_code` is invoked once the relay responds with the room code.
/// The function continues blocking until a peer joins, at which point it returns
/// the peer's public IP address.
pub fn relay_host_split<F: Fn(String)>(
    relay_addr: &str,
    udp_port: u16,
    on_code: F,
) -> std::io::Result<std::net::IpAddr> {
    use std::io::{BufRead, BufReader, Write};
    let stream = std::net::TcpStream::connect(relay_addr)?;
    let mut w = stream.try_clone()?;
    let r = BufReader::new(stream);

    write!(w, "SIGNAL:{udp_port}\n")?;
    w.flush()?;

    let mut code_received = false;
    for line in r.lines() {
        let line = line?;
        if let Some(c) = line.strip_prefix("ROOM:") {
            if !code_received {
                on_code(c.to_string());
                code_received = true;
            }
        } else if let Some(ip_str) = line.strip_prefix("PEER:") {
            let ip: std::net::IpAddr = ip_str.trim().parse()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData,
                    format!("bad PEER IP '{ip_str}': {e}")))?;
            return Ok(ip);
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof,
        "relay closed before PEER message"))
}

/// Connect to the relay server, join an existing signaling room.
/// Returns the host's UDP socket address.
/// Blocks until paired (or TCP error).
///
/// Call from a background thread — do NOT call from the main game loop.
pub fn relay_join(relay_addr: &str, code: &str)
    -> std::io::Result<std::net::SocketAddr>
{
    use std::io::{BufRead, BufReader, Write};
    let stream = std::net::TcpStream::connect(relay_addr)?;
    let mut w = stream.try_clone()?;
    let r = BufReader::new(stream);

    write!(w, "JOIN:{code}\n")?;
    w.flush()?;

    for line in r.lines() {
        let line = line?;
        if let Some(addr_str) = line.strip_prefix("PEER:") {
            let addr: std::net::SocketAddr = addr_str.trim().parse()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData,
                    format!("bad PEER addr '{addr_str}': {e}")))?;
            return Ok(addr);
        } else if line.starts_with("ERROR:") {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, line));
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof,
        "relay closed before PEER message"))
}
