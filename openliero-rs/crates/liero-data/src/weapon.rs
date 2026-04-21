//! Weapon data type — mirrors `Weapon` in `src/game/weapon.hpp`.
//!
//! All integer fields use i32 (matching C++ `int`).
//! New Sprint-7 fields are at the end with sensible defaults via `Default`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Shot type enum values (mirrors `Weapon::ShotType` in C++).
pub mod shot_type {
    pub const NORMAL:     i32 = 0;
    pub const D_TYPE_1:   i32 = 1;
    pub const STEERABLE:  i32 = 2;
    pub const D_TYPE_2:   i32 = 3;
    pub const LASER:      i32 = 4;
    pub const HOMING:     i32 = 5;
    /// Boomerang: applies constant return-force toward owner each frame.
    pub const BOOMERANG:  i32 = 6;
}

/// Dirt effect values.
pub mod dirt_effect {
    pub const NONE:    i32 = -1;
    pub const DIG:     i32 = 0;
    pub const LARPA:   i32 = 1;
    pub const DEPOSIT: i32 = 2;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Weapon {
    pub name: String,

    // --- Collision / physics ---
    pub affect_by_worm:         bool,
    pub shadow:                 bool,
    pub laser_sight:            bool,
    pub play_reload_sound:      bool,
    pub worm_explode:           bool,
    pub expl_ground:            bool,
    pub worm_collide:           bool,
    pub collide_with_objects:   bool,
    pub affect_by_explosions:   bool,
    pub loop_anim:              bool,
    pub chain_explosion:        bool,

    // --- Integer params ---
    pub detect_distance:   i32,
    pub blow_away:         i32,
    pub gravity:           i32,
    pub launch_sound:      i32,
    pub loop_sound:        i32,
    pub explo_sound:       i32,
    pub speed:             i32,
    pub add_speed:         i32,
    pub distribution:      i32,
    pub parts:             i32,
    pub recoil:            i32,
    pub mult_speed:        i32,
    pub delay:             i32,
    pub loading_time:      i32,
    pub ammo:              i32,
    pub dirt_effect:       i32,
    pub leave_shells:      i32,
    pub leave_shell_delay: i32,
    pub fire_cone:         i32,
    pub bounce:            i32,
    pub time_to_explo:     i32,
    pub time_to_explo_v:   i32,
    pub hit_damage:        i32,
    pub blood_on_hit:      i32,
    pub start_frame:       i32,
    pub num_frames:        i32,
    pub shot_type:         i32,
    pub color_bullets:     i32,
    pub splinter_amount:   i32,
    pub splinter_colour:   i32,
    pub splinter_scatter:  i32,
    pub obj_trail_delay:   i32,
    pub part_trail_type:   i32,
    pub part_trail_delay:  i32,

    // --- Optional string refs ---
    pub splinter_type:   Option<String>,
    pub obj_trail_type:  Option<String>,
    pub part_trail_obj:  Option<String>,
    pub create_on_exp:   Option<String>,

    // --- Sprint-7 extension fields ---
    pub homing_strength:       i32,
    pub attract_radius:        i32,
    pub attract_force:         i32,
    pub chain_lightning_jumps: i32,
    pub on_expire_teleport:    bool,
    pub dirt_deposit:          bool,
    pub pierce_dirt:           bool,
    /// Swap positions of owner and hit worm on impact (no damage).
    pub worm_swap:             bool,

    // --- Sprint-27 extension fields ---
    /// Black Hole Grenade: allow attract force even when embedded in terrain.
    pub attract_always:        bool,
    /// Tesla Coil: apply hit_damage to worms within attract_radius every N frames (0 = disabled).
    pub damage_area_tick:      i32,
    /// Boomerang: acceleration toward owner per frame (used with shotType=BOOMERANG).
    pub boomerang_return_force: i32,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            name:                  String::new(),
            affect_by_worm:        false,
            shadow:                false,
            laser_sight:           false,
            play_reload_sound:     false,
            worm_explode:          false,
            expl_ground:           false,
            worm_collide:          false,
            collide_with_objects:  false,
            affect_by_explosions:  false,
            loop_anim:             false,
            chain_explosion:       false,
            detect_distance:       0,
            blow_away:             0,
            gravity:               0,
            launch_sound:          0,
            loop_sound:            0,
            explo_sound:           -1,
            speed:                 100,
            add_speed:             0,
            distribution:          0,
            parts:                 1,
            recoil:                0,
            mult_speed:            100,
            delay:                 0,
            loading_time:          0,
            ammo:                  0,
            dirt_effect:           -1,
            leave_shells:          0,
            leave_shell_delay:     1,
            fire_cone:             0,
            bounce:                0,
            time_to_explo:         0,
            time_to_explo_v:       0,
            hit_damage:            0,
            blood_on_hit:          0,
            start_frame:           0,
            num_frames:            0,
            shot_type:             shot_type::NORMAL,
            color_bullets:         0,
            splinter_amount:       0,
            splinter_colour:       0,
            splinter_scatter:      0,
            obj_trail_delay:       0,
            part_trail_type:       0,
            part_trail_delay:      0,
            splinter_type:         None,
            obj_trail_type:        None,
            part_trail_obj:        None,
            create_on_exp:         None,
            homing_strength:       0,
            attract_radius:        0,
            attract_force:         0,
            chain_lightning_jumps: 0,
            on_expire_teleport:    false,
            dirt_deposit:          false,
            pierce_dirt:           false,
            worm_swap:             false,
            attract_always:        false,
            damage_area_tick:      0,
            boomerang_return_force: 0,
        }
    }
}

impl Weapon {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let src = std::str::from_utf8(raw).context("weapon cfg is not valid UTF-8")?;
        let cleaned = crate::util::strip_null_lines(src);
        toml::from_str(&cleaned).context("parsing weapon cfg")
    }
}
