//! Worm — player entity state.
//!
//! Mirrors `Worm` in `src/game/worm.hpp`.
//! All positions use fixed-point (`Fixed`). Angle is stored as an index
//! 0–127 into the 128-entry cossin table (matching C++ `aimingAngle`).
//!
//! # Reaction-force system
//! `reacts[4]` holds the accumulated push from terrain collision probes each
//! frame (UP=0, DOWN=1, LEFT=2, RIGHT=3), cleared at the start of each tick.

use crate::fixed::FixedVec;

/// Input bit-flags for one frame — same layout as the TCP lockstep protocol.
/// bits: Up/Down/Left/Right/Fire/Jump/Change/Dig
pub type WormInputBits = u32;

pub mod input {
    pub const UP:     u32 = 1 << 0;
    pub const DOWN:   u32 = 1 << 1;
    pub const LEFT:   u32 = 1 << 2;
    pub const RIGHT:  u32 = 1 << 3;
    pub const FIRE:   u32 = 1 << 4;
    pub const JUMP:   u32 = 1 << 5;
    pub const CHANGE: u32 = 1 << 6;
    pub const DIG:    u32 = 1 << 7;
    pub const ROPE:   u32 = 1 << 8;
}

/// Reaction force direction indices — mirrors C++ `Worm::RF*` enum in `worm.hpp`.
///
/// Named by the direction of the REACTION FORCE (not the probe direction):
/// - `DOWN` (0): terrain above → pushes worm down (ceiling reaction). Probes y−4.
/// - `LEFT` (1): terrain to the right → pushes worm left. Probes x+1.
/// - `UP`   (2): terrain below → pushes worm up (floor/ground reaction). Probes y+4.
/// - `RIGHT`(3): terrain to the left → pushes worm right. Probes x−1.
pub mod react {
    pub const DOWN:  usize = 0; // C++ RFDown  = 0
    pub const LEFT:  usize = 1; // C++ RFLeft  = 1
    pub const UP:    usize = 2; // C++ RFUp    = 2
    pub const RIGHT: usize = 3; // C++ RFRight = 3
}

/// Per-worm simulation state — mirrors `Worm` in `src/game/worm.hpp`.
///
/// Fields included in `fullGameChecksum()` (must stay in sync with C++):
/// `pos.x, pos.y, vel.x, vel.y, aiming_angle, health, lives, kills, timer,
/// current_weapon`
#[derive(Debug, Clone, Default)]
pub struct Worm {
    // ── Position / velocity ──────────────────────────────────────────────────
    /// World position (fixed-point Q16.16 pixels).
    pub pos:            FixedVec,
    /// Velocity (fixed-point Q16.16 pixels/frame).
    pub vel:            FixedVec,

    // ── Aiming ───────────────────────────────────────────────────────────────
    /// Aiming angle index 0–127 into `COSSIN_TABLE`.
    pub aiming_angle:   i32,

    // ── Status ───────────────────────────────────────────────────────────────
    /// Current health points.
    pub health:         i32,
    /// Lives remaining (decremented on death; game ends when 0).
    pub lives:          i32,
    /// Kill count this match.
    pub kills:          i32,
    /// Death count this match.
    pub deaths:         i32,
    /// General-purpose countdown timer (respawn delay, weapon timer, …).
    pub timer:          i32,

    // ── Weapons ──────────────────────────────────────────────────────────────
    /// Currently selected weapon slot index (0–4 typically).
    pub current_weapon: i32,
    /// Per-weapon-slot reload counters (5 slots matching C++ `wormWeapons`).
    pub reload_timers:  [i32; 5],
    /// Remaining ammo per weapon slot. -1 = unlimited.
    pub weapon_ammo:    [i32; 5],

    // ── Physics ──────────────────────────────────────────────────────────────
    /// Accumulated terrain reaction forces, one per direction (UP/DOWN/LEFT/RIGHT).
    /// Cleared each frame by `calculateReactionForce`.
    pub reacts:         [i32; 4],

    // ── Respawn ──────────────────────────────────────────────────────────────
    /// Respawn countdown: >0 = pre-respawn delay, 0 = trigger beginRespawn,
    /// -1 = doRespawning animation.  Mirrors C++ `Worm::killedTimer`.
    pub killed_timer:   i32,
    /// Animation start position for doRespawning slide (logic screen coords).
    pub logic_respawn_x: i32,
    pub logic_respawn_y: i32,

    // ── Identity ─────────────────────────────────────────────────────────────
    /// Worm slot index (0–3).
    pub index:          usize,
    /// True while the worm is alive on-field.
    pub alive:          bool,
    /// True when the worm can complete a respawn animation.
    /// Mirrors C++ `Worm::ready` — starts `true`, becomes `false` after first
    /// respawn, and is only set back by pressing Fire (no-input sims stay `false`).
    pub ready:          bool,

    // ── Game mode state ───────────────────────────────────────────────────────
    /// Team for TDM: 0 = team A, 1 = team B; -1 = FFA (no team).
    pub team_id:        i32,
    /// ZombieMode: worm was converted from human on first death.
    pub is_zombie:      bool,
    /// Juggernaut: this worm currently holds the crown (×3 HP, ×2 dmg dealt, ×½ received).
    pub is_juggernaut:  bool,
    /// BombTag: this worm is carrying the bomb.
    pub has_bomb:       bool,
    /// Permanently out of the match (lives exhausted + death animation done).
    /// Set by `Game::step()` when lives ≤ 0 and `killed_timer` hits 0.
    pub eliminated:     bool,
    /// True if this worm is controlled by bot AI instead of player input.
    pub is_bot:         bool,
    /// Bot difficulty level: 0=Easy, 1=Medium, 2=Hard.
    pub bot_difficulty: i32,

    // ── Rendering state (updated by game.step, read by renderer) ─────────────
    /// Facing direction: 0 = left, 1 = right.
    pub direction:      i32,
    /// True if the worm should be drawn this frame.
    pub visible:        bool,
    /// True while the worm is walking on ground (drives walk animation).
    pub animate:        bool,
    /// Sprite frame index (0-20): angle_frame + walk_offset.
    pub cur_frame:      i32,

    // ── Aiming velocity (Sprint-28) ────────────────────────────────────────────
    /// Angular velocity for smooth aiming (integer angle units/frame; max ~14 at full speed).
    pub aim_vel:        i32,

    // ── Charge-up weapon state (Sprint-28) ────────────────────────────────────
    /// Frames FIRE has been held for the current charge weapon (reset on fire/switch).
    pub charge_ticks:   i32,
    /// Whether FIRE was held last frame (used to detect release → fire).
    pub prev_firing:    bool,

    // ── Power-up timers (Sprint-28) ───────────────────────────────────────────
    /// Sprint Boots: remaining frames of 50% speed boost (0 = inactive).
    pub sprint_timer:   i32,
    /// Kevlar: remaining frames of 50% damage reduction (0 = inactive).
    pub kevlar_timer:   i32,
    /// Double Jump: extra mid-air jumps granted by power-up (max 1).
    pub extra_jumps:    i32,
    /// Double Jump: mid-air jumps consumed since last landing (reset on ground).
    pub jumps_used:     i32,

    // ── Input history ─────────────────────────────────────────────────────────
    /// Input bits from the previous frame (used for edge detection, e.g. weapon change).
    pub prev_inp:       u32,

    // ── Fire cone ─────────────────────────────────────────────────────────────
    /// Frames remaining before the worm can fire again (per weapon.fire_cone).
    /// Mirrors C++ `Worm::fireCone`. Decremented each frame; blocks firing when > 0.
    pub fire_cone:      i32,

    // ── Damage flash (Sprint-38) ───────────────────────────────────────────────
    /// Frames to display red tint after taking damage (0 = inactive).
    /// Set to 8 when worm takes damage; decremented each frame.
    pub damage_flash:   i32,

    // ── Shell casing delay ─────────────────────────────────────────────────────
    /// Countdown to spawn shell casing (0 = inactive).
    pub leave_shell_timer: i32,

    // ── Weapon loadout ────────────────────────────────────────────────────────
    /// Weapon type indices for each of the 5 slots.
    /// `weapon_slots[current_weapon]` → index into `Tc::weapons`.
    /// Defaults to [0, 1, 2, 3, 4] (first 5 weapons in TC order).
    pub weapon_slots:   [usize; 5],

    // ── Dig ───────────────────────────────────────────────────────────────────
    /// True when the worm is allowed to dig (LEFT+RIGHT combo).
    /// Resets to true when the combo is released. Mirrors C++ `Worm::ableToDig`.
    pub able_to_dig:    bool,

    // ── Ninja Rope ────────────────────────────────────────────────────────────
    /// True if the rope is deployed (hook flying or attached).
    pub rope_active:    bool,
    /// True if the hook has embedded in terrain.
    pub rope_attached:  bool,
    /// World position of the rope hook (Q16.16).
    pub rope_hook:      FixedVec,
    /// Velocity of the flying hook (Q16.16 pixels/frame).
    pub rope_hook_vel:  FixedVec,
    /// Max rope length in integer pixels (set when hook attaches).
    pub rope_len:       i32,
}

impl Worm {
    pub fn new(index: usize, health: i32, lives: i32) -> Self {
        Self {
            index,
            health,
            lives,
            alive:        true,
            ready:        true,
            direction:    1,
            visible:      true,
            team_id:      -1,
            able_to_dig:  true,
            weapon_slots: [0, 1, 2, 3, 4],
            weapon_ammo:  [-1; 5],  // -1 = unlimited by default
            ..Default::default()
        }
    }

    pub fn is_alive(&self) -> bool { self.alive && self.health > 0 }
}
