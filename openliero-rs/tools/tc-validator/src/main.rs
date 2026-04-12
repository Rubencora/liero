//! tc-validator — lint a TC directory and report weapon/nobject/sobject counts.
//!
//! Usage:
//!   tc-validator [TC_PATH]
//!
//! Default TC_PATH is `../../TC/openliero` (relative to the tool binary).
//!
//! Exit code 0 = all files loaded OK.
//! Exit code 1 = one or more files failed to parse.

use anyhow::Result;
use liero_data::Tc;
use std::path::PathBuf;

fn main() {
    let tc_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // CARGO_MANIFEST_DIR = openliero-rs/tools/tc-validator
            // TC/openliero is at repo root → ../../../TC/openliero
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../TC/openliero")
        });

    println!("tc-validator: loading '{}'", tc_path.display());

    match run(&tc_path) {
        Ok(()) => {
            println!("\ntc-validator: ✓ all OK");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("\ntc-validator: FAIL — {e:#}");
            std::process::exit(1);
        }
    }
}

fn run(dir: &std::path::Path) -> Result<()> {
    let tc = Tc::load(dir)?;

    let n_weapons  = tc.weapons.len();
    let n_nobjects = tc.nobjects.len();
    let n_sobjects = tc.sobjects.len();
    let n_sounds   = tc.data.types.sounds.len();

    println!();
    println!("  weapons:  {n_weapons:>4}");
    println!("  nobjects: {n_nobjects:>4}");
    println!("  sobjects: {n_sobjects:>4}");
    println!("  sounds:   {n_sounds:>4}");

    // Validate weapon cross-refs
    let mut errors = 0usize;

    for (i, w) in tc.weapons.iter().enumerate() {
        let name = &tc.data.types.weapons[i];

        if let Some(ref sobj) = w.create_on_exp {
            if !tc.data.types.sobjects.contains(sobj) {
                eprintln!("  WARN  weapon[{i}] '{name}': createOnExp = '{sobj}' not in sobjects list");
                errors += 1;
            }
        }
        if let Some(ref nobj) = w.splinter_type {
            if !tc.data.types.nobjects.contains(nobj) {
                eprintln!("  WARN  weapon[{i}] '{name}': splinterType = '{nobj}' not in nobjects list");
                errors += 1;
            }
        }
        if let Some(ref nobj) = w.obj_trail_type {
            if !tc.data.types.sobjects.contains(nobj) {
                eprintln!("  WARN  weapon[{i}] '{name}': objTrailType = '{nobj}' not in sobjects list");
                errors += 1;
            }
        }
    }

    // Validate nobject cross-refs
    for (i, n) in tc.nobjects.iter().enumerate() {
        let name = &tc.data.types.nobjects[i];

        if let Some(ref sobj) = n.create_on_exp {
            if !tc.data.types.sobjects.contains(sobj) {
                eprintln!("  WARN  nobject[{i}] '{name}': createOnExp = '{sobj}' not in sobjects list");
                errors += 1;
            }
        }
    }

    println!();
    if errors == 0 {
        println!("  cross-ref check: ✓ no broken refs");
    } else {
        println!("  cross-ref check: {errors} warning(s)");
    }

    // Print weapon list with shot type
    println!();
    println!("  Weapons ({n_weapons}):");
    for (i, w) in tc.weapons.iter().enumerate() {
        let name = &tc.data.types.weapons[i];
        let shot_label = match w.shot_type {
            0 => "normal",
            1 => "dtype1",
            2 => "steer",
            3 => "dtype2",
            4 => "laser",
            5 => "homing",
            _ => "?",
        };
        println!("    [{i:>2}] {name:<28} dmg={:>3}  shot={shot_label}", w.hit_damage);
    }

    Ok(())
}
