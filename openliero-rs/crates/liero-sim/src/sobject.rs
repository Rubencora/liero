//! SObject — active animated effect sprite runtime state.
//!
//! Mirrors `SObject` in `src/game/sobject.hpp`.
//! Type data (animation frames, timing) comes from `liero_data::SObjectType`.
//! This struct holds only per-instance simulation state.

/// Active SObject instance (Spark Object) — animated effect sprite.
#[derive(Debug, Clone)]
pub struct SObject {
    /// Top-left position in level coords (already offset by -8 from creation point).
    pub x: i32,
    pub y: i32,
    /// Index into `Tc::sobjects` (type data).
    pub type_idx: usize,
    /// Current animation frame (0..=num_frames).
    pub cur_frame: i32,
    /// Countdown until next frame advance.
    pub anim_delay_left: i32,
}
