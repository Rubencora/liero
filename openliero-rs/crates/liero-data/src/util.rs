//! Shared utilities for TC file parsing.

/// Strip lines where the value is `null` — TOML has no null type, but the
/// C++ gvl TOML writer emits `key = null` for absent optional fields.
/// Removing those lines lets serde fall back to `Default` (→ `None`).
pub fn strip_null_lines(src: &str) -> String {
    src.lines()
        .filter(|line| {
            let t = line.trim();
            // Drop any line whose value part (after `=`) is exactly `null`
            if let Some(rhs) = t.split_once('=') {
                rhs.1.trim() != "null"
            } else {
                true
            }
        })
        .flat_map(|line| [line, "\n"])
        .collect()
}
