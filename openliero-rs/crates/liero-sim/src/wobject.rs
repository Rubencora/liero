//! WObject — active projectile runtime state.
//!
//! Mirrors `WObject` in `src/game/wobject.hpp`.
//! Type data (physics params, shot type) comes from `liero_data::Weapon`.
//! This struct holds only per-instance simulation state.
//!
//! Fields included in `fullGameChecksum()`:
//! `pos.x, pos.y, vel.x, vel.y, time_left`

use crate::fixed::FixedVec;

/// Active projectile / wobject state.
#[derive(Debug, Clone, Default)]
pub struct WObject {
    // ── Position / velocity ───────────────────────────────────────────────
    /// World position (fixed-point Q16.16).
    pub pos:        FixedVec,
    /// Velocity (fixed-point Q16.16 pixels/frame).
    pub vel:        FixedVec,

    // ── Lifetime ─────────────────────────────────────────────────────────
    /// Remaining frames before expiry / explosion.
    pub time_left:  i32,

    // ── Type / ownership ─────────────────────────────────────────────────
    /// Index into `Tc::weapons` (weapon type).
    pub weapon_idx: usize,
    /// Owner worm index.
    pub owner_idx:  usize,

    // ── Animation / direction ────────────────────────────────────────────
    /// Angle index 0–127 (for steerable / homing shots) or animation frame.
    pub cur_frame:  i32,

    // ── Homing / chain lightning state ───────────────────────────────────
    /// For `STHoming`: remaining homing strength (decays each frame).
    pub homing_strength: i32,
}

impl WObject {
    pub fn new(weapon_idx: usize, owner_idx: usize, pos: FixedVec, vel: FixedVec, time_left: i32) -> Self {
        Self {
            weapon_idx,
            owner_idx,
            pos,
            vel,
            time_left,
            ..Default::default()
        }
    }
}
