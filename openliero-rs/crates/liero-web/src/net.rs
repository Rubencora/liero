//! WebSocket-based rollback netcode for browser multiplayer.
//!
//! `WsSession` is a port of `liero_net::RollbackSession` that uses
//! `web_sys::WebSocket` instead of a `UdpSocket`.  The 25-byte `NetPacket`
//! format is identical, so a browser client can play against a native desktop
//! client through the relay.
//!
//! # Usage
//! 1. Open a `WebSocket` connection to the relay and complete the handshake
//!    (send `CREATE:name` or `JOIN:code`, wait for `PAIRED`).
//! 2. Call `WsSession::new(ws, local_player)`.
//! 3. On every frame call `session.step(game, local_raw_input)` instead of
//!    `game.step(...)`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

use liero_sim::game::{Game, GameSnapshot};

// ── Packet ────────────────────────────────────────────────────────────────────

const PACKET_LEN: usize = 25;
const INPUT_RING:  usize = 256;
const SNAP_RING:   usize = 16;

/// Maximum frames of prediction before stalling.
pub const MAX_ROLLBACK: usize = 16;

#[derive(Debug, Clone, Copy)]
struct NetPacket {
    frame:      u32,
    player_idx: u8,
    inputs:     [u32; 3],
    checksum:   u64,
}

impl NetPacket {
    fn encode(&self) -> [u8; PACKET_LEN] {
        let mut buf = [0u8; PACKET_LEN];
        buf[0..4].copy_from_slice(&self.frame.to_le_bytes());
        buf[4] = self.player_idx;
        buf[5..9].copy_from_slice(&self.inputs[0].to_le_bytes());
        buf[9..13].copy_from_slice(&self.inputs[1].to_le_bytes());
        buf[13..17].copy_from_slice(&self.inputs[2].to_le_bytes());
        buf[17..25].copy_from_slice(&self.checksum.to_le_bytes());
        buf
    }

    fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < PACKET_LEN { return None; }
        Some(Self {
            frame:      u32::from_le_bytes(buf[0..4].try_into().ok()?),
            player_idx: buf[4],
            inputs: [
                u32::from_le_bytes(buf[5..9].try_into().ok()?),
                u32::from_le_bytes(buf[9..13].try_into().ok()?),
                u32::from_le_bytes(buf[13..17].try_into().ok()?),
            ],
            checksum: u64::from_le_bytes(buf[17..25].try_into().ok()?),
        })
    }
}

// ── WsSession ─────────────────────────────────────────────────────────────────

/// Rollback netcode session backed by a WebSocket relay connection.
pub struct WsSession {
    ws:           WebSocket,
    recv_queue:   Rc<RefCell<VecDeque<Vec<u8>>>>,
    /// Keep closures alive for the lifetime of the session.
    _callbacks:   Vec<Closure<dyn FnMut(MessageEvent)>>,

    local_player:  usize,
    remote_player: usize,

    pub local_frame:    u32,
    confirmed_frame:    u32,

    local_inputs:  Box<[u32; INPUT_RING]>,
    remote_inputs: Box<[u32; INPUT_RING]>,

    snapshots: Vec<Option<GameSnapshot>>,

    pub desync_detected: bool,
    pub desync_frame:    u32,
    pub desync_message:  String,
}

impl WsSession {
    /// Create a new session wrapping an already-paired WebSocket.
    ///
    /// `local_player`: 0 for the host, 1 for the joiner.
    pub fn new(ws: WebSocket, local_player: usize) -> Self {
        let recv_queue: Rc<RefCell<VecDeque<Vec<u8>>>> =
            Rc::new(RefCell::new(VecDeque::new()));

        // Install onmessage callback: push binary frames into recv_queue.
        let queue = Rc::clone(&recv_queue);
        let on_msg = Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(ab) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let bytes = js_sys::Uint8Array::new(&ab).to_vec();
                queue.borrow_mut().push_back(bytes);
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        ws.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));

        let snapshots = (0..SNAP_RING).map(|_| None).collect();

        Self {
            ws,
            recv_queue,
            _callbacks: vec![on_msg],
            local_player,
            remote_player: 1 - local_player,
            local_frame: 0,
            confirmed_frame: 0,
            local_inputs:  Box::new([0u32; INPUT_RING]),
            remote_inputs: Box::new([0u32; INPUT_RING]),
            snapshots,
            desync_detected: false,
            desync_frame: 0,
            desync_message: String::new(),
        }
    }

    /// Advance by one frame with rollback support.
    ///
    /// Returns `true` if the step was executed, `false` if stalled (waiting
    /// for remote to catch up).
    pub fn step(&mut self, game: &mut Game, local_raw: u32) -> bool {
        if self.should_stall() {
            self.drain_packets(game);
            return false;
        }

        // 1. Save snapshot.
        let snap_slot = self.local_frame as usize % SNAP_RING;
        self.snapshots[snap_slot] = Some(game.save_snapshot());

        // 2. Drain incoming packets (may trigger rollback + re-sim).
        self.drain_packets(game);

        // 3. Record local input; predict remote (last confirmed).
        let li = self.local_frame as usize % INPUT_RING;
        self.local_inputs[li] = local_raw;

        let pred_remote = self.remote_inputs[self.confirmed_frame as usize % INPUT_RING];
        let mut inputs = [0u32; 4];
        inputs[self.local_player]  = local_raw;
        inputs[self.remote_player] = pred_remote;
        game.step(&inputs);

        // 4. Send packet to peer via relay.
        self.send_packet(game);

        self.local_frame += 1;
        true
    }

    pub fn should_stall(&self) -> bool {
        let ahead = self.local_frame.saturating_sub(self.confirmed_frame + 1);
        ahead as usize >= MAX_ROLLBACK
    }

    // ── Private ───────────────────────────────────────────────────────────────

    fn drain_packets(&mut self, game: &mut Game) {
        let msgs: Vec<Vec<u8>> = self.recv_queue.borrow_mut().drain(..).collect();
        for bytes in msgs {
            if let Some(pkt) = NetPacket::decode(&bytes) {
                self.handle_packet(game, &pkt);
            }
        }
    }

    fn handle_packet(&mut self, game: &mut Game, pkt: &NetPacket) {
        if pkt.player_idx as usize != self.remote_player { return; }

        let mut rollback_needed = false;
        let mut rollback_to = pkt.frame;

        for (i, &inp) in pkt.inputs.iter().enumerate() {
            let f = pkt.frame.saturating_sub(2 - i as u32);
            if f == 0 { continue; }

            let slot = f as usize % INPUT_RING;

            if f > self.confirmed_frame {
                self.remote_inputs[slot] = inp;
                self.confirmed_frame = f;
            } else if self.remote_inputs[slot] != inp {
                self.remote_inputs[slot] = inp;
                if f < rollback_to { rollback_to = f; }
                rollback_needed = true;
            }
        }

        // Desync detection.
        if !self.desync_detected
            && pkt.frame + 1 == self.local_frame
            && pkt.checksum != game.checksum()
        {
            self.desync_detected = true;
            self.desync_frame    = pkt.frame;
            self.desync_message  = format!("Game state desync at frame {}", pkt.frame);
        }

        // Rollback + re-simulate if needed.
        if rollback_needed && rollback_to < self.local_frame {
            let age = self.local_frame - rollback_to;
            if age as usize > SNAP_RING { return; }

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
        let encoded = pkt.encode();
        let _ = self.ws.send_with_u8_array(&encoded);
    }
}
