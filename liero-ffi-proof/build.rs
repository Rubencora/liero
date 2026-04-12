use std::env;
use std::path::PathBuf;

fn main() {
    // Path to the pre-built libliero_sim.a (built by CMake in the build/ directory).
    // Can be overridden with LIERO_BUILD_DIR environment variable.
    let build_dir = env::var("LIERO_BUILD_DIR")
        .unwrap_or_else(|_| "../build".to_string());

    let build_path = PathBuf::from(&build_dir)
        .canonicalize()
        .expect("build directory not found — run `cmake --build build --target liero_sim` first");

    println!("cargo:rustc-link-search=native={}", build_path.display());
    println!("cargo:rustc-link-lib=static=liero_sim");

    // libliero_sim.a is compiled from C++ and requires the C++ runtime.
    // On macOS: libc++ (LLVM). On Linux: libstdc++.
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }

    // Re-run if the static lib changes.
    println!(
        "cargo:rerun-if-changed={}",
        build_path.join("libliero_sim.a").display()
    );
    println!("cargo:rerun-if-changed=../src/game/sim_c_api.h");
}
