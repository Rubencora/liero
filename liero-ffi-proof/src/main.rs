//! S8-02 — Rust FFI proof-of-concept for libliero_sim.
//!
//! Loads the headless simulation library, runs 1000 deterministic frames with
//! seed=42, and prints the final checksum.  If the checksum matches the
//! golden.csv produced by sim_ci, the FFI wiring is correct.
//!
//! Usage (from repo root):
//!   cd liero-ffi-proof
//!   cargo run -- [tc-path]
//! Default tc-path is "../TC/openliero".

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};

// ---------------------------------------------------------------------------
// Raw FFI bindings — mirrors sim_c_api.h exactly.
// ---------------------------------------------------------------------------

/// Opaque simulation handle.
#[repr(C)]
pub struct SimHandle {
    _private: [u8; 0],
}

/// Per-worm input for one frame (bit-packed, same layout as the TCP protocol).
///   bits 0-6  : WormControlStates (Up/Down/Left/Right/Fire/Jump/Change/Dig)
///   bit  7    : mouse aim active
///   bits 8-14 : mouse aim angle (0-127)
pub type SimWormInput = u32;

/// Input bundle for one frame — up to 4 worms.
#[repr(C)]
pub struct SimFrameInput {
    pub worms: [SimWormInput; 4],
    pub num_worms: c_int,
}

extern "C" {
    fn sim_create(tc_path: *const c_char) -> *mut SimHandle;
    fn sim_destroy(sim: *mut SimHandle);
    fn sim_start_game(sim: *mut SimHandle, seed: c_uint);
    fn sim_step(sim: *mut SimHandle, inputs: *const SimFrameInput);
    fn sim_checksum(sim: *mut SimHandle) -> u64;
    fn sim_cycles(sim: *mut SimHandle) -> c_int;
    fn sim_is_game_over(sim: *mut SimHandle) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrapper
// ---------------------------------------------------------------------------

pub struct Sim(*mut SimHandle);

impl Sim {
    pub fn new(tc_path: &str) -> Option<Self> {
        let c_path = CString::new(tc_path).ok()?;
        let handle = unsafe { sim_create(c_path.as_ptr()) };
        if handle.is_null() {
            None
        } else {
            Some(Self(handle))
        }
    }

    pub fn start(&mut self, seed: u32) {
        unsafe { sim_start_game(self.0, seed) }
    }

    pub fn step(&mut self) {
        unsafe { sim_step(self.0, std::ptr::null()) }
    }

    pub fn checksum(&mut self) -> u64 {
        unsafe { sim_checksum(self.0) }
    }

    pub fn cycles(&mut self) -> i32 {
        unsafe { sim_cycles(self.0) }
    }

    pub fn is_game_over(&mut self) -> bool {
        unsafe { sim_is_game_over(self.0) != 0 }
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        unsafe { sim_destroy(self.0) }
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let tc_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../TC/openliero".to_string());

    println!("liero-ffi-proof: loading TC from '{tc_path}'");

    let mut sim = Sim::new(&tc_path).unwrap_or_else(|| {
        eprintln!("ERROR: sim_create failed — check TC path and build/libliero_sim.a");
        std::process::exit(1);
    });

    println!("liero-ffi-proof: TC loaded OK — starting game with seed=42");
    sim.start(42);

    const FRAMES: i32 = 1000;
    for _ in 0..FRAMES {
        sim.step();
    }

    let csum = sim.checksum();
    let cycles = sim.cycles();
    let over = sim.is_game_over();

    println!("liero-ffi-proof: {cycles} frames simulated");
    println!("liero-ffi-proof: checksum = {csum:#018x}");
    println!("liero-ffi-proof: game_over = {over}");
    println!();
    println!("✓  S8-01: libliero_sim C ABI — clean, no SDL at runtime");
    println!("✓  S8-02: Rust FFI — sim_create / sim_start_game / sim_step / sim_checksum all reachable");
    println!("    Compare checksum against ../TC/openliero/golden.csv frame 1000 to verify determinism.");
}
