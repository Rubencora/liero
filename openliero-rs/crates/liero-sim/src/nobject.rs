//! NObject — non-owner particle runtime state.
//!
//! Mirrors the runtime part of `NObject` in `src/game/sobject.hpp`.
//! Type data (damage, physics params) comes from `liero_data::NObjectType`.
//! This struct holds only per-instance simulation state.

use crate::fixed::FixedVec;

/// Live NObject (particle / non-owner projectile).
///
/// Fields match the per-instance state tracked in C++ `NObject::process()`.
#[derive(Debug, Clone, Default)]
pub struct NObject {
    /// World position (fixed-point Q16.16).
    pub pos:       FixedVec,
    /// Velocity (fixed-point Q16.16 pixels/frame).
    pub vel:       FixedVec,
    /// Index into `Tc::nobjects` (type).
    pub nobj_type: usize,
    /// Remaining lifetime in frames (counts down to 0 → expire/explode).
    pub time_left: i32,
    /// Current sprite/colour frame (for renderer).
    /// If `NObjectType::start_frame > 0`: index into small_sprites.
    /// If `start_frame <= 0`: used as a palette colour index directly.
    pub cur_frame: i32,
}

impl NObject {
    pub fn new(nobj_type: usize, pos: FixedVec, vel: FixedVec, time_left: i32) -> Self {
        Self { pos, vel, nobj_type, time_left, ..Default::default() }
    }
}
