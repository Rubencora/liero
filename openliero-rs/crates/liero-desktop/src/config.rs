//! Persistent user configuration — stored as TOML.
//!
//! On macOS  : `~/Library/Application Support/openliero/config.toml`
//! On Linux  : `~/.config/openliero/config.toml`
//! On Windows: `%APPDATA%\openliero\config.toml`

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Path to the TC directory (can be overridden by CLI arg).
    pub tc_path:   String,
    /// Last selected local game mode index.
    pub last_mode: usize,
    /// Last used host port string.
    pub host_port: String,
    /// CRT scanlines effect enabled.
    pub scanlines: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tc_path:   "TC/openliero".to_string(),
            last_mode: 0,
            host_port: "7777".to_string(),
            scanlines: false,
        }
    }
}

impl Config {
    /// OS-appropriate config file path.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("openliero")
            .join("config.toml")
    }

    /// Load from disk; falls back to [`Default`] on any error.
    pub fn load() -> Self {
        let path = Self::path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist to disk.  Silently ignores write errors.
    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, s);
        }
    }
}
