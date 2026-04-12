/// build.rs — Link against C++ libliero_sim.a.
///
/// Looks for the static library in these locations (in order):
///   1. $LIERO_SIM_LIB  (env var — CI/CD override)
///   2. <repo-root>/build/libliero_sim.a
///   3. <repo-root>/build_debug/libliero_sim.a
///
/// The C++ library requires libc++ (macOS/Linux) or libstdc++ (Linux alt).
fn main() {
    // ── Find libliero_sim.a ───────────────────────────────────────────────
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // tools/replay-diff → openliero-rs → repo root
    let repo_root = manifest.join("../../..");

    let lib_dir = if let Ok(dir) = std::env::var("LIERO_SIM_LIB") {
        std::path::PathBuf::from(dir)
    } else {
        // Prefer release build, fall back to debug.
        let release = repo_root.join("build");
        let debug   = repo_root.join("build_debug");
        if release.join("libliero_sim.a").exists() {
            release
        } else if debug.join("libliero_sim.a").exists() {
            debug
        } else {
            // Print a helpful error; the build will fail at link time.
            eprintln!(
                "cargo:warning=replay-diff: libliero_sim.a not found. \
                 Build the C++ project first, or set LIERO_SIM_LIB."
            );
            release
        }
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=liero_sim");

    // ── C++ runtime ───────────────────────────────────────────────────────
    // macOS uses libc++; Linux typically uses libstdc++.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=c++");
    #[cfg(not(target_os = "macos"))]
    println!("cargo:rustc-link-lib=stdc++");

    // Rebuild if the library changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LIERO_SIM_LIB");
    println!("cargo:rerun-if-changed={}", lib_dir.join("libliero_sim.a").display());
}
