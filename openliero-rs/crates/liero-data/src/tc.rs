//! Top-level TC config (`tc.cfg`) — types, physics constants, texts, hacks.
//!
//! The `[types]` section lists all weapon/nobject/sobject/sound names; these
//! are used to load the individual `.cfg` files from their subdirectories.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;


// ---------------------------------------------------------------------------
// [types]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TcTypes {
    pub sounds:   Vec<String>,
    pub weapons:  Vec<String>,
    pub nobjects: Vec<String>,
    pub sobjects: Vec<String>,
}

// ---------------------------------------------------------------------------
// [constants] — nested arrays
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcBonus {
    pub timer:  i32,
    #[serde(rename = "timerV")]
    pub timer_v: i32,
    pub frame:  i32,
    pub sobj:   Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcTexture {
    pub mframe:    i32,
    pub rframe:    i32,
    pub sframe:    i32,
    pub ndrawback: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcColorAnim {
    pub from: i32,
    pub to:   i32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcAiParam {
    pub on:  i32,
    pub off: i32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcAiParams {
    pub up:     TcAiParam,
    pub down:   TcAiParam,
    pub left:   TcAiParam,
    pub right:  TcAiParam,
    pub fire:   TcAiParam,
    pub change: TcAiParam,
    pub jump:   TcAiParam,
}

// ---------------------------------------------------------------------------
// [constants] — physics values (all PascalCase in TOML)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct TcConstants {
    // Palette material table (256 entries, index = palette colour).
    // Key is lowercase "materials" in tc.cfg — override the PascalCase rename.
    #[serde(rename = "materials")]
    pub materials: Vec<u8>,

    // Ninja rope
    #[serde(rename = "NRInitialLength")]
    pub nr_initial_length: i32,
    #[serde(rename = "NRAttachLength")]
    pub nr_attach_length: i32,
    #[serde(rename = "NRMinLength")]
    pub nr_min_length: i32,
    #[serde(rename = "NRMaxLength")]
    pub nr_max_length: i32,
    #[serde(rename = "NRPullVel")]
    pub nr_pull_vel: i32,
    #[serde(rename = "NRReleaseVel")]
    pub nr_release_vel: i32,
    #[serde(rename = "NRColourBegin")]
    pub nr_colour_begin: i32,
    #[serde(rename = "NRColourEnd")]
    pub nr_colour_end: i32,
    #[serde(rename = "NRThrowVelX")]
    pub nr_throw_vel_x: i32,
    #[serde(rename = "NRThrowVelY")]
    pub nr_throw_vel_y: i32,
    #[serde(rename = "NRForceShlX")]
    pub nr_force_shl_x: i32,
    #[serde(rename = "NRForceDivX")]
    pub nr_force_div_x: i32,
    #[serde(rename = "NRForceShlY")]
    pub nr_force_shl_y: i32,
    #[serde(rename = "NRForceDivY")]
    pub nr_force_div_y: i32,
    #[serde(rename = "NRForceLenShl")]
    pub nr_force_len_shl: i32,
    #[serde(rename = "NinjaropeGravity")]
    pub ninjrarope_gravity: i32,

    // Worm physics
    pub min_bounce_up:    i32,
    pub min_bounce_down:  i32,
    pub min_bounce_left:  i32,
    pub min_bounce_right: i32,
    pub worm_gravity:     i32,
    pub walk_vel_left:    i32,
    pub max_vel_left:     i32,
    pub walk_vel_right:   i32,
    pub max_vel_right:    i32,
    pub jump_force:       i32,
    pub worm_fric_mult:   i32,
    pub worm_fric_div:    i32,
    pub worm_float_level: i32,
    pub worm_float_power: i32,

    // Aiming
    pub max_aim_vel_left:  i32,
    pub aim_acc_left:      i32,
    pub max_aim_vel_right: i32,
    pub aim_acc_right:     i32,
    pub aim_fric_mult:     i32,
    pub aim_fric_div:      i32,
    pub aim_max_right:     i32,
    pub aim_min_right:     i32,
    pub aim_max_left:      i32,
    pub aim_min_left:      i32,

    // Bonuses
    pub bonus_gravity:      i32,
    pub bonus_bounce_mul:   i32,
    pub bonus_bounce_div:   i32,
    pub bonus_flicker_time: i32,
    pub bonus_explode_risk: i32,
    pub bonus_health_var:   i32,
    pub bonus_min_health:   i32,
    pub bonus_drop_chance:  i32,
    pub bonus_spawn_rect_x: i32,
    pub bonus_spawn_rect_y: i32,
    pub bonus_spawn_rect_w: i32,
    pub bonus_spawn_rect_h: i32,

    // Worm spawn
    pub worm_min_spawn_dist_last:  i32,
    pub worm_min_spawn_dist_enemy: i32,
    pub worm_spawn_rect_x: i32,
    pub worm_spawn_rect_y: i32,
    pub worm_spawn_rect_w: i32,
    pub worm_spawn_rect_h: i32,

    // Blood
    pub first_blood_colour: i32,
    pub num_blood_colours:  i32,
    pub blood_step_up:      i32,
    pub blood_step_down:    i32,
    pub blood_limit:        i32,

    // Fall damage (per direction)
    pub fall_damage_right: i32,
    pub fall_damage_left:  i32,
    pub fall_damage_down:  i32,
    pub fall_damage_up:    i32,

    // Misc
    pub laser_weapon:              i32,
    pub b_obj_gravity:             i32,
    pub splinter_larpa_vel_div:    i32,
    pub splinter_crackler_vel_div: i32,
    pub rem_exp_object:            i32,

    // Nested sub-tables
    #[serde(rename = "bonuses")]
    pub bonuses:    Vec<TcBonus>,
    #[serde(rename = "textures")]
    pub textures:   Vec<TcTexture>,
    #[serde(rename = "colorAnim")]
    pub color_anim: Vec<TcColorAnim>,
    pub aiparams:   TcAiParams,
}

// ---------------------------------------------------------------------------
// [texts]
// ---------------------------------------------------------------------------

/// Game UI strings (PascalCase keys in TOML).
/// Only the fields used by liero-sim or liero-render are modelled;
/// the rest are consumed by `#[serde(flatten)]` fallback.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct TcTexts {
    pub killed_msg:            String,
    pub committed_suicide_msg: String,
    pub press_fire:            String,
    pub kills:                 String,
    pub lives:                 String,
}

// ---------------------------------------------------------------------------
// [hacks]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct TcHacks {
    pub fall_damage:       bool,
    pub bonus_reload_only: bool,
    pub bonus_spawn_rect:  bool,
    pub bonus_only_health: bool,
    pub bonus_only_weapon: bool,
    pub bonus_disable:     bool,
    pub worm_float:        bool,
    pub rem_exp:           bool,
    pub signed_recoil:     bool,
    pub air_jump:          bool,
    pub multi_jump:        bool,
}

// ---------------------------------------------------------------------------
// Top-level TcData
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TcData {
    pub types:     TcTypes,
    pub constants: TcConstants,
    pub texts:     TcTexts,
    pub hacks:     TcHacks,
}

impl TcData {
    pub fn load(dir: &Path) -> Result<Self> {
        let p = dir.join("tc.cfg");
        let raw = std::fs::read(&p)
            .with_context(|| format!("reading {}", p.display()))?;
        Self::from_bytes(&raw).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let src = std::str::from_utf8(raw).context("tc.cfg is not valid UTF-8")?;
        let normalized = normalize_tc_cfg(src);
        toml::from_str(&normalized).context("parsing tc.cfg")
    }
}

/// Normalize tc.cfg for strict TOML v1.0 parsers.
///
/// The C++ TOML writer emits `[[constants.*]]` array-of-tables and
/// `[constants.aiparams.*]` sub-tables *around* (and before) the
/// `[constants]` scalar key blocks.  TOML v1.0 forbids re-opening an
/// implicitly-created super-table, so we rebuild the document:
///
/// 1. Merge the bodies of ALL `[constants]` sections into one block.
/// 2. Emit that merged `[constants]` block first.
/// 3. Then emit all `[[constants.*]]` array-of-tables.
/// 4. Then emit all `[constants.*]` sub-table sections (aiparams, etc.).
/// 5. Finally the remaining top-level sections ([types], [texts], [hacks]).
fn normalize_tc_cfg(src: &str) -> String {
    // Split the document into sections.  Each section starts at a line
    // that begins with `[` (table header) and extends to the next such line.
    let mut sections: Vec<(String, String)> = Vec::new(); // (header, body)
    let mut current_header = String::new();
    let mut current_body   = String::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            sections.push((
                std::mem::take(&mut current_header),
                std::mem::take(&mut current_body),
            ));
            current_header = line.to_string();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    sections.push((current_header, current_body));

    // Partition sections by kind
    let mut preamble:          Vec<(String, String)> = Vec::new(); // before constants
    let mut constants_bodies:  Vec<String>           = Vec::new(); // merged [constants] bodies
    let mut constants_arrays:  Vec<(String, String)> = Vec::new(); // [[constants.*]]
    let mut constants_subs:    Vec<(String, String)> = Vec::new(); // [constants.*] (not [constants])
    let mut after_constants:   Vec<(String, String)> = Vec::new(); // [texts], [hacks], etc.

    let mut seen_constants = false;

    for (h, b) in sections {
        let t = h.trim();
        if t == "[constants]" {
            // Merge body; skip header (we emit one combined header later)
            constants_bodies.push(b);
            seen_constants = true;
        } else if t.starts_with("[[constants.") {
            constants_arrays.push((h, b));
            seen_constants = true;
        } else if t.starts_with("[constants.") {
            constants_subs.push((h, b));
            seen_constants = true;
        } else if seen_constants && !t.is_empty() {
            after_constants.push((h, b));
        } else {
            preamble.push((h, b));
        }
    }

    // Reassemble in valid TOML order:
    //   preamble → [constants] (merged) → [[constants.*]] → [constants.*] → rest
    let mut out = String::new();

    for (h, b) in preamble {
        if !h.is_empty() { out.push_str(&h); out.push('\n'); }
        out.push_str(&b);
    }

    // Merged [constants] scalar block
    out.push_str("[constants]\n");
    for body in constants_bodies {
        out.push_str(&body);
    }

    // [[constants.*]] array-of-tables
    for (h, b) in constants_arrays {
        out.push_str(&h); out.push('\n');
        out.push_str(&b);
    }

    // [constants.*] sub-tables (aiparams, etc.)
    for (h, b) in constants_subs {
        out.push_str(&h); out.push('\n');
        out.push_str(&b);
    }

    for (h, b) in after_constants {
        if !h.is_empty() { out.push_str(&h); out.push('\n'); }
        out.push_str(&b);
    }

    out
}
