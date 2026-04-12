//! NObjectType — nobject (non-owner projectile / particle) data.
//! Mirrors `NObject` type fields in `src/game/sobject.hpp`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NObjectType {
    pub worm_explode:         bool,
    pub expl_ground:          bool,
    pub worm_destroy:         bool,
    pub draw_on_map:          bool,
    pub affect_by_explosions: bool,
    pub blood_trail:          bool,

    pub detect_distance:  i32,
    pub gravity:          i32,
    pub speed:            i32,
    pub speed_v:          i32,
    pub distribution:     i32,
    pub blow_away:        i32,
    pub bounce:           i32,
    pub hit_damage:       i32,
    pub blood_on_hit:     i32,
    pub start_frame:      i32,
    pub num_frames:       i32,
    pub color_bullets:    i32,
    pub dirt_effect:      i32,
    pub splinter_amount:  i32,
    pub splinter_colour:  i32,
    pub blood_trail_delay:i32,
    pub leave_obj_delay:  i32,
    pub time_to_explo:    i32,
    pub time_to_explo_v:  i32,

    pub create_on_exp: Option<String>,
    pub splinter_type: Option<String>,
    pub leave_obj:     Option<String>,
}

impl Default for NObjectType {
    fn default() -> Self {
        Self {
            worm_explode:         false,
            expl_ground:          false,
            worm_destroy:         false,
            draw_on_map:          false,
            affect_by_explosions: false,
            blood_trail:          false,
            detect_distance:      0,
            gravity:              0,
            speed:                0,
            speed_v:              0,
            distribution:         0,
            blow_away:            0,
            bounce:               0,
            hit_damage:           0,
            blood_on_hit:         0,
            start_frame:          0,
            num_frames:           0,
            color_bullets:        0,
            dirt_effect:          -1,
            splinter_amount:      0,
            splinter_colour:      0,
            blood_trail_delay:    0,
            leave_obj_delay:      0,
            time_to_explo:        0,
            time_to_explo_v:      0,
            create_on_exp:        None,
            splinter_type:        None,
            leave_obj:            None,
        }
    }
}

impl NObjectType {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let src = std::str::from_utf8(raw).context("nobject cfg is not valid UTF-8")?;
        let cleaned = crate::util::strip_null_lines(src);
        toml::from_str(&cleaned).context("parsing nobject cfg")
    }
}
