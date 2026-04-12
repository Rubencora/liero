//! SObjectType — static (screen) explosion / effect data.
//! Mirrors `SObject` type fields in `src/game/sobject.hpp`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SObjectType {
    pub shadow:      bool,

    pub start_sound: i32,
    pub num_sounds:  i32,
    pub anim_delay:  i32,
    pub start_frame: i32,
    pub num_frames:  i32,
    pub detect_range:i32,
    pub damage:      i32,
    pub blow_away:   i32,
    pub shake:       i32,
    pub flash:       i32,
    pub dirt_effect: i32,
}

impl Default for SObjectType {
    fn default() -> Self {
        Self {
            shadow:       false,
            start_sound:  -1,
            num_sounds:   0,
            anim_delay:   1,
            start_frame:  0,
            num_frames:   1,
            detect_range: 0,
            damage:       0,
            blow_away:    0,
            shake:        0,
            flash:        0,
            dirt_effect:  -1,
        }
    }
}

impl SObjectType {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let src = std::str::from_utf8(raw).context("sobject cfg is not valid UTF-8")?;
        toml::from_str(src).context("parsing sobject cfg")
    }
}
