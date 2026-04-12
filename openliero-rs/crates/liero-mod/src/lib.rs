//! `liero-mod` — Lua 5.4 modding layer for OpenLiero.
//!
//! Exposes TC weapons, nobjects, and sobjects to a Lua script that can freely
//! mutate them. After the script runs, all changes are written back into the
//! `Tc` struct.
//!
//! # Usage
//! ```no_run
//! use liero_data::Tc;
//! use liero_mod::apply_mod;
//! use std::path::Path;
//!
//! let mut tc = Tc::load(Path::new("TC/openliero")).unwrap();
//! let lua_src = std::fs::read_to_string("TC/openliero/mod.lua").unwrap();
//! apply_mod(&mut tc, &lua_src).unwrap();
//! ```
//!
//! # Lua API
//!
//! Three globals are available in the script:
//! - `weapons`  — array (1-indexed) of weapon tables
//! - `nobjects` — array (1-indexed) of nobject tables
//! - `sobjects` — array (1-indexed) of sobject tables
//!
//! Each table has all the same field names as the Rust struct (snake_case).
//! Mutating a field in the script immediately affects the TC when the script
//! returns. String-ref fields (`splinter_type`, `obj_trail_type`, etc.) accept
//! either a string value or `nil` (to clear).
//!
//! Example `mod.lua`:
//! ```lua
//! -- Double speed of all weapons
//! for _, w in ipairs(weapons) do
//!     w.speed = w.speed * 2
//! end
//!
//! -- Boost the first nobject's gravity
//! nobjects[1].gravity = nobjects[1].gravity + 500
//! ```

use anyhow::{Context, Result};
use liero_data::{NObjectType, SObjectType, Tc, Weapon};
use mlua::prelude::*;

/// Convert a `LuaError` to an `anyhow::Error` without requiring `Send + Sync`.
fn lua_err(e: LuaError) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

/// Apply a Lua mod script to a loaded `Tc`, mutating it in place.
///
/// The script receives three globals — `weapons`, `nobjects`, `sobjects` —
/// each an array of tables with read-write fields. Any changes are written back
/// into `tc` after the script completes.
pub fn apply_mod(tc: &mut Tc, lua_source: &str) -> Result<()> {
    let lua = Lua::new();

    // Build the three global arrays.
    let weapons_tbl  = build_weapons_table(&lua, &tc.weapons).map_err(lua_err)?;
    let nobjects_tbl = build_nobjects_table(&lua, &tc.nobjects).map_err(lua_err)?;
    let sobjects_tbl = build_sobjects_table(&lua, &tc.sobjects).map_err(lua_err)?;

    lua.globals().set("weapons",  weapons_tbl).map_err(lua_err)?;
    lua.globals().set("nobjects", nobjects_tbl).map_err(lua_err)?;
    lua.globals().set("sobjects", sobjects_tbl).map_err(lua_err)?;

    // Execute the script.
    lua.load(lua_source)
        .set_name("mod.lua")
        .exec()
        .map_err(lua_err)
        .context("executing mod.lua")?;

    // Read back changes.
    let weapons_tbl:  LuaTable = lua.globals().get("weapons").map_err(lua_err)?;
    let nobjects_tbl: LuaTable = lua.globals().get("nobjects").map_err(lua_err)?;
    let sobjects_tbl: LuaTable = lua.globals().get("sobjects").map_err(lua_err)?;

    for (i, weapon) in tc.weapons.iter_mut().enumerate() {
        let tbl: LuaTable = weapons_tbl.get(i + 1).map_err(lua_err)?;
        pull_weapon(weapon, &tbl)
            .map_err(lua_err)
            .with_context(|| format!("reading back weapon[{}]", i))?;
    }
    for (i, nobj) in tc.nobjects.iter_mut().enumerate() {
        let tbl: LuaTable = nobjects_tbl.get(i + 1).map_err(lua_err)?;
        pull_nobject(nobj, &tbl)
            .map_err(lua_err)
            .with_context(|| format!("reading back nobject[{}]", i))?;
    }
    for (i, sobj) in tc.sobjects.iter_mut().enumerate() {
        let tbl: LuaTable = sobjects_tbl.get(i + 1).map_err(lua_err)?;
        pull_sobject(sobj, &tbl)
            .map_err(lua_err)
            .with_context(|| format!("reading back sobject[{}]", i))?;
    }

    Ok(())
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn bool_to_lua(b: bool) -> i32 { if b { 1 } else { 0 } }
fn lua_to_bool(v: i32) -> bool  { v != 0 }

/// Read an optional string from a Lua table field (nil → None).
fn get_opt_str(tbl: &LuaTable, key: &str) -> LuaResult<Option<String>> {
    match tbl.get::<LuaValue>(key)? {
        LuaValue::Nil | LuaValue::Boolean(false) => Ok(None),
        LuaValue::String(s) => Ok(Some(s.to_str()?.to_owned())),
        other => Err(LuaError::RuntimeError(
            format!("field `{key}` must be a string or nil, got {:?}", other)
        )),
    }
}

// ─── weapons ─────────────────────────────────────────────────────────────────

fn build_weapons_table(lua: &Lua, weapons: &[Weapon]) -> LuaResult<LuaTable> {
    let arr = lua.create_table()?;
    for (i, w) in weapons.iter().enumerate() {
        arr.set(i + 1, weapon_to_table(lua, w)?)?;
    }
    Ok(arr)
}

fn weapon_to_table(lua: &Lua, w: &Weapon) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("name",                  w.name.as_str())?;
    t.set("affect_by_worm",        bool_to_lua(w.affect_by_worm))?;
    t.set("shadow",                bool_to_lua(w.shadow))?;
    t.set("laser_sight",           bool_to_lua(w.laser_sight))?;
    t.set("play_reload_sound",     bool_to_lua(w.play_reload_sound))?;
    t.set("worm_explode",          bool_to_lua(w.worm_explode))?;
    t.set("expl_ground",           bool_to_lua(w.expl_ground))?;
    t.set("worm_collide",          bool_to_lua(w.worm_collide))?;
    t.set("collide_with_objects",  bool_to_lua(w.collide_with_objects))?;
    t.set("affect_by_explosions",  bool_to_lua(w.affect_by_explosions))?;
    t.set("loop_anim",             bool_to_lua(w.loop_anim))?;
    t.set("chain_explosion",       bool_to_lua(w.chain_explosion))?;
    t.set("detect_distance",       w.detect_distance)?;
    t.set("blow_away",             w.blow_away)?;
    t.set("gravity",               w.gravity)?;
    t.set("launch_sound",          w.launch_sound)?;
    t.set("loop_sound",            w.loop_sound)?;
    t.set("explo_sound",           w.explo_sound)?;
    t.set("speed",                 w.speed)?;
    t.set("add_speed",             w.add_speed)?;
    t.set("distribution",          w.distribution)?;
    t.set("parts",                 w.parts)?;
    t.set("recoil",                w.recoil)?;
    t.set("mult_speed",            w.mult_speed)?;
    t.set("delay",                 w.delay)?;
    t.set("loading_time",          w.loading_time)?;
    t.set("ammo",                  w.ammo)?;
    t.set("dirt_effect",           w.dirt_effect)?;
    t.set("leave_shells",          w.leave_shells)?;
    t.set("leave_shell_delay",     w.leave_shell_delay)?;
    t.set("fire_cone",             w.fire_cone)?;
    t.set("bounce",                w.bounce)?;
    t.set("time_to_explo",         w.time_to_explo)?;
    t.set("time_to_explo_v",       w.time_to_explo_v)?;
    t.set("hit_damage",            w.hit_damage)?;
    t.set("blood_on_hit",          w.blood_on_hit)?;
    t.set("start_frame",           w.start_frame)?;
    t.set("num_frames",            w.num_frames)?;
    t.set("shot_type",             w.shot_type)?;
    t.set("color_bullets",         w.color_bullets)?;
    t.set("splinter_amount",       w.splinter_amount)?;
    t.set("splinter_colour",       w.splinter_colour)?;
    t.set("splinter_scatter",      w.splinter_scatter)?;
    t.set("obj_trail_delay",       w.obj_trail_delay)?;
    t.set("part_trail_type",       w.part_trail_type)?;
    t.set("part_trail_delay",      w.part_trail_delay)?;
    t.set("splinter_type",         w.splinter_type.as_deref())?;
    t.set("obj_trail_type",        w.obj_trail_type.as_deref())?;
    t.set("part_trail_obj",        w.part_trail_obj.as_deref())?;
    t.set("create_on_exp",         w.create_on_exp.as_deref())?;
    t.set("homing_strength",       w.homing_strength)?;
    t.set("attract_radius",        w.attract_radius)?;
    t.set("attract_force",         w.attract_force)?;
    t.set("chain_lightning_jumps", w.chain_lightning_jumps)?;
    t.set("on_expire_teleport",    bool_to_lua(w.on_expire_teleport))?;
    t.set("dirt_deposit",          bool_to_lua(w.dirt_deposit))?;
    t.set("pierce_dirt",           bool_to_lua(w.pierce_dirt))?;
    Ok(t)
}

fn pull_weapon(w: &mut Weapon, t: &LuaTable) -> LuaResult<()> {
    w.name                  = t.get::<String>("name")?;
    w.affect_by_worm        = lua_to_bool(t.get("affect_by_worm")?);
    w.shadow                = lua_to_bool(t.get("shadow")?);
    w.laser_sight           = lua_to_bool(t.get("laser_sight")?);
    w.play_reload_sound     = lua_to_bool(t.get("play_reload_sound")?);
    w.worm_explode          = lua_to_bool(t.get("worm_explode")?);
    w.expl_ground           = lua_to_bool(t.get("expl_ground")?);
    w.worm_collide          = lua_to_bool(t.get("worm_collide")?);
    w.collide_with_objects  = lua_to_bool(t.get("collide_with_objects")?);
    w.affect_by_explosions  = lua_to_bool(t.get("affect_by_explosions")?);
    w.loop_anim             = lua_to_bool(t.get("loop_anim")?);
    w.chain_explosion       = lua_to_bool(t.get("chain_explosion")?);
    w.detect_distance       = t.get("detect_distance")?;
    w.blow_away             = t.get("blow_away")?;
    w.gravity               = t.get("gravity")?;
    w.launch_sound          = t.get("launch_sound")?;
    w.loop_sound            = t.get("loop_sound")?;
    w.explo_sound           = t.get("explo_sound")?;
    w.speed                 = t.get("speed")?;
    w.add_speed             = t.get("add_speed")?;
    w.distribution          = t.get("distribution")?;
    w.parts                 = t.get("parts")?;
    w.recoil                = t.get("recoil")?;
    w.mult_speed            = t.get("mult_speed")?;
    w.delay                 = t.get("delay")?;
    w.loading_time          = t.get("loading_time")?;
    w.ammo                  = t.get("ammo")?;
    w.dirt_effect           = t.get("dirt_effect")?;
    w.leave_shells          = t.get("leave_shells")?;
    w.leave_shell_delay     = t.get("leave_shell_delay")?;
    w.fire_cone             = t.get("fire_cone")?;
    w.bounce                = t.get("bounce")?;
    w.time_to_explo         = t.get("time_to_explo")?;
    w.time_to_explo_v       = t.get("time_to_explo_v")?;
    w.hit_damage            = t.get("hit_damage")?;
    w.blood_on_hit          = t.get("blood_on_hit")?;
    w.start_frame           = t.get("start_frame")?;
    w.num_frames            = t.get("num_frames")?;
    w.shot_type             = t.get("shot_type")?;
    w.color_bullets         = t.get("color_bullets")?;
    w.splinter_amount       = t.get("splinter_amount")?;
    w.splinter_colour       = t.get("splinter_colour")?;
    w.splinter_scatter      = t.get("splinter_scatter")?;
    w.obj_trail_delay       = t.get("obj_trail_delay")?;
    w.part_trail_type       = t.get("part_trail_type")?;
    w.part_trail_delay      = t.get("part_trail_delay")?;
    w.splinter_type         = get_opt_str(t, "splinter_type")?;
    w.obj_trail_type        = get_opt_str(t, "obj_trail_type")?;
    w.part_trail_obj        = get_opt_str(t, "part_trail_obj")?;
    w.create_on_exp         = get_opt_str(t, "create_on_exp")?;
    w.homing_strength       = t.get("homing_strength")?;
    w.attract_radius        = t.get("attract_radius")?;
    w.attract_force         = t.get("attract_force")?;
    w.chain_lightning_jumps = t.get("chain_lightning_jumps")?;
    w.on_expire_teleport    = lua_to_bool(t.get("on_expire_teleport")?);
    w.dirt_deposit          = lua_to_bool(t.get("dirt_deposit")?);
    w.pierce_dirt           = lua_to_bool(t.get("pierce_dirt")?);
    Ok(())
}

// ─── nobjects ────────────────────────────────────────────────────────────────

fn build_nobjects_table(lua: &Lua, nobjects: &[NObjectType]) -> LuaResult<LuaTable> {
    let arr = lua.create_table()?;
    for (i, n) in nobjects.iter().enumerate() {
        arr.set(i + 1, nobject_to_table(lua, n)?)?;
    }
    Ok(arr)
}

fn nobject_to_table(lua: &Lua, n: &NObjectType) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("worm_explode",          bool_to_lua(n.worm_explode))?;
    t.set("expl_ground",           bool_to_lua(n.expl_ground))?;
    t.set("worm_destroy",          bool_to_lua(n.worm_destroy))?;
    t.set("draw_on_map",           bool_to_lua(n.draw_on_map))?;
    t.set("affect_by_explosions",  bool_to_lua(n.affect_by_explosions))?;
    t.set("blood_trail",           bool_to_lua(n.blood_trail))?;
    t.set("detect_distance",       n.detect_distance)?;
    t.set("gravity",               n.gravity)?;
    t.set("speed",                 n.speed)?;
    t.set("speed_v",               n.speed_v)?;
    t.set("distribution",          n.distribution)?;
    t.set("blow_away",             n.blow_away)?;
    t.set("bounce",                n.bounce)?;
    t.set("hit_damage",            n.hit_damage)?;
    t.set("blood_on_hit",          n.blood_on_hit)?;
    t.set("start_frame",           n.start_frame)?;
    t.set("num_frames",            n.num_frames)?;
    t.set("color_bullets",         n.color_bullets)?;
    t.set("dirt_effect",           n.dirt_effect)?;
    t.set("splinter_amount",       n.splinter_amount)?;
    t.set("splinter_colour",       n.splinter_colour)?;
    t.set("blood_trail_delay",     n.blood_trail_delay)?;
    t.set("leave_obj_delay",       n.leave_obj_delay)?;
    t.set("time_to_explo",         n.time_to_explo)?;
    t.set("time_to_explo_v",       n.time_to_explo_v)?;
    t.set("create_on_exp",         n.create_on_exp.as_deref())?;
    t.set("splinter_type",         n.splinter_type.as_deref())?;
    t.set("leave_obj",             n.leave_obj.as_deref())?;
    Ok(t)
}

fn pull_nobject(n: &mut NObjectType, t: &LuaTable) -> LuaResult<()> {
    n.worm_explode         = lua_to_bool(t.get("worm_explode")?);
    n.expl_ground          = lua_to_bool(t.get("expl_ground")?);
    n.worm_destroy         = lua_to_bool(t.get("worm_destroy")?);
    n.draw_on_map          = lua_to_bool(t.get("draw_on_map")?);
    n.affect_by_explosions = lua_to_bool(t.get("affect_by_explosions")?);
    n.blood_trail          = lua_to_bool(t.get("blood_trail")?);
    n.detect_distance      = t.get("detect_distance")?;
    n.gravity              = t.get("gravity")?;
    n.speed                = t.get("speed")?;
    n.speed_v              = t.get("speed_v")?;
    n.distribution         = t.get("distribution")?;
    n.blow_away            = t.get("blow_away")?;
    n.bounce               = t.get("bounce")?;
    n.hit_damage           = t.get("hit_damage")?;
    n.blood_on_hit         = t.get("blood_on_hit")?;
    n.start_frame          = t.get("start_frame")?;
    n.num_frames           = t.get("num_frames")?;
    n.color_bullets        = t.get("color_bullets")?;
    n.dirt_effect          = t.get("dirt_effect")?;
    n.splinter_amount      = t.get("splinter_amount")?;
    n.splinter_colour      = t.get("splinter_colour")?;
    n.blood_trail_delay    = t.get("blood_trail_delay")?;
    n.leave_obj_delay      = t.get("leave_obj_delay")?;
    n.time_to_explo        = t.get("time_to_explo")?;
    n.time_to_explo_v      = t.get("time_to_explo_v")?;
    n.create_on_exp        = get_opt_str(t, "create_on_exp")?;
    n.splinter_type        = get_opt_str(t, "splinter_type")?;
    n.leave_obj            = get_opt_str(t, "leave_obj")?;
    Ok(())
}

// ─── sobjects ────────────────────────────────────────────────────────────────

fn build_sobjects_table(lua: &Lua, sobjects: &[SObjectType]) -> LuaResult<LuaTable> {
    let arr = lua.create_table()?;
    for (i, s) in sobjects.iter().enumerate() {
        arr.set(i + 1, sobject_to_table(lua, s)?)?;
    }
    Ok(arr)
}

fn sobject_to_table(lua: &Lua, s: &SObjectType) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("shadow",       bool_to_lua(s.shadow))?;
    t.set("start_sound",  s.start_sound)?;
    t.set("num_sounds",   s.num_sounds)?;
    t.set("anim_delay",   s.anim_delay)?;
    t.set("start_frame",  s.start_frame)?;
    t.set("num_frames",   s.num_frames)?;
    t.set("detect_range", s.detect_range)?;
    t.set("damage",       s.damage)?;
    t.set("blow_away",    s.blow_away)?;
    t.set("shake",        s.shake)?;
    t.set("flash",        s.flash)?;
    t.set("dirt_effect",  s.dirt_effect)?;
    Ok(t)
}

fn pull_sobject(s: &mut SObjectType, t: &LuaTable) -> LuaResult<()> {
    s.shadow       = lua_to_bool(t.get("shadow")?);
    s.start_sound  = t.get("start_sound")?;
    s.num_sounds   = t.get("num_sounds")?;
    s.anim_delay   = t.get("anim_delay")?;
    s.start_frame  = t.get("start_frame")?;
    s.num_frames   = t.get("num_frames")?;
    s.detect_range = t.get("detect_range")?;
    s.damage       = t.get("damage")?;
    s.blow_away    = t.get("blow_away")?;
    s.shake        = t.get("shake")?;
    s.flash        = t.get("flash")?;
    s.dirt_effect  = t.get("dirt_effect")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use liero_data::{NObjectType, SObjectType, TcData, Weapon};

    /// Build a minimal fake Tc with one weapon for testing.
    fn fake_tc() -> Tc {
        let mut w = Weapon::default();
        w.name  = "TestGun".to_string();
        w.speed = 100;
        w.parts = 1;

        Tc {
            data:          TcData::default(),
            weapons:       vec![w],
            nobjects:      vec![NObjectType::default()],
            sobjects:      vec![SObjectType::default()],
            sounds:        vec![],
            large_sprites: vec![],
            small_sprites: vec![],
            worm_sprites:  vec![],
            palette:       vec![],
        }
    }

    #[test]
    fn modify_weapon_speed() {
        let mut tc = fake_tc();
        apply_mod(&mut tc, "weapons[1].speed = 999").unwrap();
        assert_eq!(tc.weapons[0].speed, 999);
    }

    #[test]
    fn double_all_weapon_parts() {
        let mut tc = fake_tc();
        apply_mod(&mut tc, "for _, w in ipairs(weapons) do w.parts = w.parts * 3 end").unwrap();
        assert_eq!(tc.weapons[0].parts, 3);
    }

    #[test]
    fn noop_script_preserves_data() {
        let mut tc = fake_tc();
        let orig_speed = tc.weapons[0].speed;
        apply_mod(&mut tc, "-- no-op").unwrap();
        assert_eq!(tc.weapons[0].speed, orig_speed);
    }

    #[test]
    fn syntax_error_returns_err() {
        let mut tc = fake_tc();
        let result = apply_mod(&mut tc, "this is not valid lua @@@@");
        assert!(result.is_err());
    }
}
