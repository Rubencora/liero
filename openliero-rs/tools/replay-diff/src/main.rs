//! replay-diff — Frame-accurate checksum comparison.
//!
//! Runs the **C++ `libliero_sim`** and the **Rust `liero-sim`** in lockstep for
//! N frames and reports the first diverging frame.
//!
//! # Usage
//!
//!   replay-diff [TC_PATH] [N_FRAMES] [SEED]
//!
//! Defaults: TC_PATH = ../TC/openliero (relative to CWD),
//!           N_FRAMES = 1000, SEED = 42.
//!
//! # Sync strategy
//! After `sim_start_game`, the C++ sim exports its full state via
//! `sim_export_state` (rand.x/c, cycles, per-worm fields).  The Rust sim
//! is then initialised from that snapshot so both sims begin at identical
//! values without needing a Rust port of the level generator.
//!
//! # Exit codes
//!   0 — all N frames match
//!   1 — at least one mismatch (printed to stderr)
//!   2 — startup error (bad TC path, null handle, etc.)

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};
use std::path::PathBuf;
use liero_sim::game::{ExportedState, ExportedWormState, MAX_WORMS};

// ---------------------------------------------------------------------------
// FFI — mirrors sim_c_api.h
// ---------------------------------------------------------------------------

#[repr(C)]
struct SimHandle { _opaque: [u8; 0] }

#[repr(C)]
struct SimFrameInput {
    worms:     [c_uint; 4],
    num_worms: c_int,
}

/// Must exactly mirror `sim_worm_state_t` in `sim_c_api.h`.
#[repr(C)]
struct FfiWormState {
    pos_x: i32, pos_y: i32,
    vel_x: i32, vel_y: i32,
    aiming_angle:   i32,
    health:         i32,
    lives:          i32,
    kills:          i32,
    timer:          i32,
    current_weapon: i32,
    killed_timer:   i32,
    visible:        i32,
}

/// Must exactly mirror `sim_state_t` in `sim_c_api.h`.
#[repr(C)]
struct FfiState {
    rand_x:    u32,
    rand_c:    u32,
    cycles:    i32,
    num_worms: i32,
    worms:     [FfiWormState; 4],
}

extern "C" {
    fn sim_create(tc_path: *const c_char) -> *mut SimHandle;
    fn sim_destroy(sim: *mut SimHandle);
    fn sim_start_game(sim: *mut SimHandle, seed: c_uint);
    fn sim_step(sim: *mut SimHandle, inputs: *const SimFrameInput);
    fn sim_checksum(sim: *mut SimHandle) -> u64;
    fn sim_cycles(sim: *mut SimHandle) -> c_int;
    fn sim_export_state(sim: *mut SimHandle, out: *mut FfiState);
    fn sim_export_level(sim: *mut SimHandle,
                        out_pixels: *mut u8,
                        out_width:  *mut i32,
                        out_height: *mut i32);
    fn sim_get_rand_x(sim: *mut SimHandle) -> u32;
    fn sim_get_rand_c(sim: *mut SimHandle) -> u32;
    fn sim_get_bonus_count(sim: *mut SimHandle) -> i32;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_tc_path() -> String { "../TC/openliero".to_owned() }

fn ffi_to_exported(ffi: &FfiState) -> ExportedState {
    let mut worms: [ExportedWormState; MAX_WORMS] = Default::default();
    for i in 0..MAX_WORMS {
        let fw = &ffi.worms[i];
        worms[i] = ExportedWormState {
            pos_x: fw.pos_x, pos_y: fw.pos_y,
            vel_x: fw.vel_x, vel_y: fw.vel_y,
            aiming_angle:   fw.aiming_angle,
            health:         fw.health,
            lives:          fw.lives,
            kills:          fw.kills,
            timer:          fw.timer,
            current_weapon: fw.current_weapon,
            killed_timer:   fw.killed_timer,
            visible:        fw.visible,
        };
    }
    ExportedState { rand_x: ffi.rand_x, rand_c: ffi.rand_c, cycles: ffi.cycles,
                    num_worms: ffi.num_worms, worms }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let tc_path_str: String = args.next().unwrap_or_else(default_tc_path);
    let n_frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let seed: u32       = args.next().and_then(|s| s.parse().ok()).unwrap_or(42);

    println!("replay-diff: TC='{tc_path_str}'  frames={n_frames}  seed={seed}");
    println!();

    // ── C++ sim ──────────────────────────────────────────────────────────
    let tc_cstr = CString::new(tc_path_str.as_str())
        .map_err(|e| anyhow::anyhow!("CString: {e}"))?;

    let cpp_sim = unsafe { sim_create(tc_cstr.as_ptr()) };
    if cpp_sim.is_null() {
        anyhow::bail!("sim_create() returned null — bad TC path '{tc_path_str}'?");
    }
    unsafe { sim_start_game(cpp_sim, seed) };

    // Export C++ initial state.
    let mut ffi_state = std::mem::MaybeUninit::<FfiState>::zeroed();
    unsafe { sim_export_state(cpp_sim, ffi_state.as_mut_ptr()) };
    let ffi_state = unsafe { ffi_state.assume_init() };
    let exported  = ffi_to_exported(&ffi_state);

    println!("  C++ initial state: rand.x={} rand.c={} cycles={} worms={}",
             exported.rand_x, exported.rand_c, exported.cycles, exported.num_worms);
    for i in 0..(exported.num_worms as usize) {
        let w = &exported.worms[i];
        println!("    worm[{i}]: pos=({},{}) vel=({},{}) angle={} hp={} lives={} kills={} timer={} weapon={} killed_timer={} visible={}",
            w.pos_x, w.pos_y, w.vel_x, w.vel_y, w.aiming_angle,
            w.health, w.lives, w.kills, w.timer, w.current_weapon,
            w.killed_timer, w.visible);
    }

    // ── Rust sim ─────────────────────────────────────────────────────────
    let tc_path_abs = PathBuf::from(&tc_path_str).canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot open TC '{}': {e}", tc_path_str))?;

    let tc = liero_data::Tc::load(&tc_path_abs)
        .map_err(|e| anyhow::anyhow!("loading TC: {e}"))?;

    let mut rust_sim = liero_sim::game::Game::new(tc, exported.num_worms as usize, seed);

    // Export C++ level pixels and sync to Rust sim.
    let level_size = (504 * 350) as usize; // worst case; will be overwritten
    let mut level_pixels: Vec<u8> = vec![0u8; level_size];
    let mut lw: i32 = 504;
    let mut lh: i32 = 350;
    unsafe {
        sim_export_level(cpp_sim,
                         level_pixels.as_mut_ptr(),
                         &mut lw as *mut i32,
                         &mut lh as *mut i32);
    }
    level_pixels.truncate((lw * lh) as usize);
    println!("  Level: {}×{} pixels", lw, lh);
    rust_sim.load_level_pixels(&level_pixels, lw as u32, lh as u32);

    // Sync Rust sim to the C++ post-start_game state.
    rust_sim.load_exported_state(&exported);

    // ── Comparison loop ──────────────────────────────────────────────────
    let empty_input = SimFrameInput { worms: [0; 4], num_worms: exported.num_worms };

    let mut mismatches = 0usize;
    let mut first_mismatch: Option<usize> = None;

    // Pre-allocate level pixel buffer for per-frame re-sync.
    let level_buf_size = (lw * lh) as usize;
    let mut sync_pixels: Vec<u8> = vec![0u8; level_buf_size];

    println!();
    for frame in 1..=n_frames {
        // Sync Rust level from C++ BEFORE either sim steps this frame.
        // Both sims must process the same terrain during frame N's logic.
        // C++ modifies terrain during processFrame (drawDirtEffect, explosions, etc.).
        // Re-syncing here ensures Rust uses the same end-of-previous-frame terrain.
        unsafe {
            sim_export_level(cpp_sim,
                             sync_pixels.as_mut_ptr(),
                             &mut lw as *mut i32,
                             &mut lh as *mut i32);
        }
        rust_sim.load_level_pixels(&sync_pixels[..(lw * lh) as usize], lw as u32, lh as u32);

        // Advance C++ sim.
        unsafe { sim_step(cpp_sim, &empty_input) };
        let cpp_csum  = unsafe { sim_checksum(cpp_sim) };
        let cpp_cycle = unsafe { sim_cycles(cpp_sim) };

        // Advance Rust sim.
        rust_sim.step(&[0u32; 4][..exported.num_worms as usize]);
        let rust_csum = rust_sim.checksum();

        // Log bonus state for frames near first mismatch.
        if first_mismatch.is_some() && frame <= first_mismatch.unwrap() + 3 {
            let cpp_rx = unsafe { sim_get_rand_x(cpp_sim) };
            let cpp_bn = unsafe { sim_get_bonus_count(cpp_sim) };
            let rust_bn = rust_sim.bonuses.iter().filter(|b| b.is_some()).count() as i32;
            eprintln!("  [frame {:>4}] rand_x cpp={:>12} rust={:>12}  bonuses: cpp={} rust={}  rust_timers={:?}",
                frame, cpp_rx, rust_sim.rand.x, cpp_bn, rust_bn,
                rust_sim.bonuses.iter().filter_map(|b| b.as_ref().map(|bb| bb.timer)).collect::<Vec<_>>());
        }

        if cpp_csum != rust_csum {
            if mismatches == 0 {
                first_mismatch = Some(frame);
                // Dump worm states + rand on first divergence.
                let cpp_rx = unsafe { sim_get_rand_x(cpp_sim) };
                let cpp_rc = unsafe { sim_get_rand_c(cpp_sim) };
                eprintln!("  [frame {frame}] rand: cpp=({cpp_rx},{cpp_rc}) rust=({},{})",
                    rust_sim.rand.x, rust_sim.rand.c);
                let mut ffi_st = std::mem::MaybeUninit::<FfiState>::zeroed();
                unsafe { sim_export_state(cpp_sim, ffi_st.as_mut_ptr()) };
                let ffi_st = unsafe { ffi_st.assume_init() };
                for i in 0..(exported.num_worms as usize) {
                    let cw = &ffi_st.worms[i];
                    let rw = &rust_sim.worms[i];
                    eprintln!("  worm[{i}]:");
                    for (label, cv, rv) in [
                        ("pos_x", cw.pos_x, rw.pos.x.0),
                        ("pos_y", cw.pos_y, rw.pos.y.0),
                        ("vel_x", cw.vel_x, rw.vel.x.0),
                        ("vel_y", cw.vel_y, rw.vel.y.0),
                        ("angle", cw.aiming_angle, rw.aiming_angle),
                        ("health", cw.health, rw.health),
                        ("lives", cw.lives, rw.lives),
                        ("timer", cw.timer, rw.timer),
                        ("weapon", cw.current_weapon, rw.current_weapon),
                        ("killed_timer", cw.killed_timer, rw.killed_timer),
                    ] {
                        let sym = if cv == rv { "=" } else { "!" };
                        if cv != rv {
                            eprintln!("    {sym} {label}: cpp={cv} rust={rv}");
                        }
                    }
                    let alive_cpp = cw.visible != 0;
                    if alive_cpp != rw.alive {
                        eprintln!("    ! alive: cpp={alive_cpp} rust={}", rw.alive);
                    }
                }
            }
            if mismatches < 10 {
                eprintln!("  MISMATCH frame {:>4}  (cpp_cycle={})  cpp={:020}  rust={:020}",
                          frame, cpp_cycle, cpp_csum, rust_csum);
            } else if mismatches == 10 {
                eprintln!("  ... (further mismatches suppressed)");
            }
            mismatches += 1;
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────
    unsafe { sim_destroy(cpp_sim) };

    // ── Report ───────────────────────────────────────────────────────────
    println!();
    if mismatches == 0 {
        println!("replay-diff: ✓  {n_frames} frames — all checksums match");
        std::process::exit(0);
    } else {
        let first = first_mismatch.unwrap_or(0);
        println!("replay-diff: ✗  {mismatches} / {n_frames} frames mismatch  \
                  (first divergence at frame {first})");
        std::process::exit(1);
    }
}
