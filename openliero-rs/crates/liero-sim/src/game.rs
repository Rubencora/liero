//! Game — top-level simulation state container.
//!
//! Holds all worms, active projectiles, particles, and the level.
//! `Game::step()` is the main entry point for one simulation frame.
//!
//! # R0 gate
//! `game_checksum()` must produce bit-identical results to the C++
//! `fullGameChecksum()` for the same seed and input sequence.
//!
//! # Checksum algorithm (from `src/game/game.cpp:947-999`)
//! FNV-1a 64-bit, **word-at-a-time** (each `mix` XORs a `u32`, then multiplies).
//! Field order: `rand.x, cycles`, then per-worm, per-wobject, per-nobject.

use crate::fixed::{Fixed, FixedVec};
use crate::level::{Level, WIDTH, HEIGHT};
use crate::levelgen;
use crate::material::Material;
use crate::math::{COSSIN_TABLE, vector_length};
use crate::nobject::NObject;
use crate::rand::Mwc;
use crate::sobject::SObject;
use crate::worm::{Worm, react};
use crate::wobject::WObject;
use liero_data::Tc;
use liero_data::weapon;

/// Maximum simultaneous worms (4-player support).
pub const MAX_WORMS: usize = 4;

// ── Game mode types ───────────────────────────────────────────────────────────

// ── Rollback snapshot ─────────────────────────────────────────────────────────

fn isqrt32(n: i32) -> i32 {
    if n <= 0 { return 0; }
    let mut x = 1i32 << ((32 - n.leading_zeros()) / 2);
    loop {
        let y = (x + n / x) >> 1;
        if y >= x { break; }
        x = y;
    }
    x
}

/// Full simulation state snapshot for rollback netcode.
///
/// Contains every mutable field of `Game` except `tc` (constant) and
/// `tc_materials` (derived from `tc`).  A 16-slot ring buffer of these
/// snapshots costs ~16 × 180 KB ≈ 2.9 MB — acceptable for desktop.
#[derive(Clone)]
pub struct GameSnapshot {
    pub rand:           crate::rand::Mwc,
    pub cycles:         i32,
    pub worms:          Vec<crate::worm::Worm>,
    pub wobjects:       Vec<crate::wobject::WObject>,
    pub nobjects:       Vec<crate::nobject::NObject>,
    pub sobjects:       Vec<crate::sobject::SObject>,
    pub bonuses:        Vec<Option<Bonus>>,
    pub floating_damages: Vec<FloatingDamage>,
    pub level_pixels:   Vec<u8>,
    pub koth_ticks:     [i32; 4],
    pub bomb_timer:     i32,
    pub dirt_scores:    [i32; 4],
    pub tag_worm:       Option<usize>,
    pub worm_checkpoint: [i32; 4],
    pub worm_laps:      [i32; 4],
    pub round_end_timer: i32,
    pub pending_result: Option<GameResult>,
    pub screen_flash:   i32,
    pub viewport_shake: [i32; 4],
}

// ── Sound events ─────────────────────────────────────────────────────────────

/// A sound trigger emitted by the simulation.
///
/// `Game::sound_events` is filled during `step()` and drained by the host
/// (desktop / web) which maps events to audio samples and plays them.
/// No audio dependency in `liero-sim` — just plain data.
#[derive(Debug, Clone)]
pub enum SoundEvent {
    /// Worm fired a weapon; `weapon_idx` selects the weapon's start_sound.
    WeaponFire { weapon_idx: usize },
    /// A sobject explosion was triggered at the given position.
    Explosion { sobj_idx: usize },
    /// A worm took damage (hurt sound; `worm_idx` for stereo pan, unused for now).
    Hurt { worm_idx: usize },
    /// A worm died.
    Death { worm_idx: usize },
    /// A worm collected a bonus.
    BonusCollect,
    /// Worm hit a wall/terrain hard (bounce sound, C++ index 14).
    WormBounce { worm_idx: usize },
    /// Worm respawned (C++ index 21).
    WormRespawn { worm_idx: usize },
    /// Weapon reload finished (C++ index 24).
    WeaponReloaded { worm_idx: usize },
    /// Ninja rope thrown (C++ index 5).
    RopeThrown { worm_idx: usize },
}

/// Which win-condition and rule-set to use for this match.
#[derive(Debug, Clone, PartialEq)]
pub enum GameMode {
    /// Everyone for themselves.  Worm(s) with lives > 0 wins; no respawn when eliminated.
    LastManStanding,
    /// Two teams (A=worms 0,2; B=worms 1,3).  Last team with a living worm wins.
    TeamDeathmatch,
    /// Hold the central zone.  First worm to accumulate `time_limit` in-zone ticks wins.
    KingOfHill { time_limit: i32 },
    /// One worm carries the bomb.  On hit → bomb transfers; fuse = 0 → carrier explodes.
    BombTag { fuse: i32 },
    /// First death converts worm to zombie (half HP, always respawn).  Last human wins.
    ZombieMode,
    /// One worm is the Juggernaut (×3 HP, ×2 dmg dealt, ×½ received).  Crown passes on death.
    Juggernaut,
    /// Deposit dirt with Dirt Cannon — highest score when time runs out wins.
    DirtWar { time_limit: i32 },
    /// Kill Race: first worm to reach `frag_limit` kills wins. Infinite respawns.
    KillRace { frag_limit: i32 },
    /// Gun Game: kill an enemy to advance to the next weapon slot. First to complete all weapons wins.
    GunGame,
    /// Race mode: navigate through checkpoints and complete laps. First to complete all laps wins.
    Race {
        checkpoints: Vec<(i32, i32)>,
        laps: i32,
    },
    /// Game of Tag: last worm to get a kill has their timer increment each frame. First to reach time_limit wins.
    GameOfTag { time_limit: i32 },
    /// Scales of Justice: dealing damage heals the attacker. Gain a life if healed worm HP exceeds max.
    ScalesOfJustice,
}

impl Default for GameMode {
    fn default() -> Self { Self::LastManStanding }
}

/// Outcome of a concluded match.
#[derive(Debug, Clone, PartialEq)]
pub enum GameResult {
    /// A single worm won — holds the worm index.
    WormWins(usize),
    /// A team won — holds the team_id (0 or 1).
    TeamWins(i32),
    /// Match ended with no winner.
    Draw,
}

/// Per-worm state inside [`ExportedState`].
/// Field layout mirrors `sim_worm_state_t` in `sim_c_api.h`.
#[derive(Debug, Clone, Default)]
pub struct ExportedWormState {
    pub pos_x: i32, pub pos_y: i32,
    pub vel_x: i32, pub vel_y: i32,
    pub aiming_angle:   i32,
    pub health:         i32,
    pub lives:          i32,
    pub kills:          i32,
    pub timer:          i32,
    pub current_weapon: i32,
    pub killed_timer:   i32,
    pub visible:        i32,
}

/// Full simulation snapshot exported by `sim_export_state` in C++.
/// Used by the `replay-diff` harness to sync the Rust sim initial state.
#[derive(Debug, Clone)]
pub struct ExportedState {
    pub rand_x:    u32,
    pub rand_c:    u32,
    pub cycles:    i32,
    pub num_worms: i32,
    pub worms:     [ExportedWormState; MAX_WORMS],
}

/// A bonus pickup on the level (weapon crate or health pack).
///
/// All positional fields are fixed-point Q16.16, matching C++ `Bonus::x/y/velY`.
/// `frame` = 0 (weapon) or 1 (health).
#[derive(Debug, Clone, Default)]
pub struct Bonus {
    pub x:      i32,   // fixed-point
    pub y:      i32,   // fixed-point
    pub vel_y:  i32,   // fixed-point
    pub frame:  i32,   // 0 = weapon, 1 = health
    pub timer:  i32,   // countdown; ≤ 0 → bonus expires
    pub weapon: i32,   // weapon index (valid when frame == 0)
    pub used:   bool,  // set true when collected by a worm
}

/// Floating damage indicator — displays damage amount above worm, drifts upward and fades.
#[derive(Debug, Clone)]
pub struct FloatingDamage {
    pub x:      i32,   // world x in fixed-point Q16.16
    pub y:      i32,   // world y in fixed-point Q16.16
    pub amount: i32,   // damage amount to display
    pub timer:  i32,   // frames remaining (starts at 40)
    pub vel_y:  i32,   // upward velocity in fixed-point Q16.16 (negative = up)
}

/// Simulation state — one independent game instance.
pub struct Game {
    // ── Core ─────────────────────────────────────────────────────────────────
    /// Terrain pixel map.
    pub level:    Level,
    /// Live worms (player entities).
    pub worms:    Vec<Worm>,
    /// Active projectiles (player-fired wobjects).
    pub wobjects: Vec<WObject>,
    /// Active particles (non-owner nobjects).
    pub nobjects: Vec<NObject>,
    /// Active animated effect sprites (sobjects).
    pub sobjects: Vec<SObject>,
    /// Active pickup bonuses (weapon crates / health packs).
    /// Uses pool semantics: `None` slots are freed bonuses (do not remove/swap).
    pub bonuses:  Vec<Option<Bonus>>,
    /// Floating damage indicators (damage numbers drifting upward).
    pub floating_damages: Vec<FloatingDamage>,
    /// Loaded TC (weapons, nobject/sobject types, constants).
    pub tc:       Tc,

    // ── Determinism state ────────────────────────────────────────────────────
    /// MWC RNG — mirrors `game.rand` in C++.
    pub rand:     Mwc,
    /// Global frame / cycle counter — mirrors `game.cycles` in C++.
    pub cycles:   i32,
    /// RNG seed used in `Game::new()` — saved here for replay reconstruction.
    pub seed:     u32,

    // ── Settings (defaults, no UI) ───────────────────────────────────────────
    /// Maximum simultaneous bonuses (default 4, mirrors C++ `Settings::maxBonuses`).
    pub max_bonuses: i32,
    /// Worm starting health (default 100).
    pub worm_health: i32,
    /// Worm starting lives (default 3).
    pub worm_lives: i32,
    /// Blood particle amount percentage (default 100, 0-100 range).
    pub blood_pct: i32,
    /// Weapon loading time factor as percentage (default 100; 50 = half speed).
    pub loading_time_factor: i32,
    /// Enable shadows (default true).
    pub shadows_enabled: bool,
    /// Allow friendly fire even in team modes (default true).
    pub friendly_fire: bool,
    /// Random weapon loadout per round (default false).
    pub random_weapons: bool,

    // ── Audio events (filled by step(), drained by host) ────────────────────
    /// Sound events produced during the last `step()` call.
    /// The desktop/web host drains this after each step and plays samples.
    pub sound_events: Vec<SoundEvent>,

    // ── Game mode ────────────────────────────────────────────────────────────
    /// Active game mode — controls win condition, respawn rules, and special mechanics.
    pub mode:         GameMode,
    /// Per-worm in-zone tick count for King of the Hill.
    pub koth_ticks:   [i32; 4],
    /// BombTag: frames remaining on the fuse.  0 when BombTag is not active.
    pub bomb_timer:   i32,
    /// DirtWar: dirt tiles deposited per worm.
    pub dirt_scores:  [i32; 4],
    /// GameOfTag: which worm is "it" (last killer).
    pub tag_worm:     Option<usize>,
    /// Race mode: current checkpoint index per worm.
    pub worm_checkpoint: [i32; 4],
    /// Race mode: completed laps per worm.
    pub worm_laps:    [i32; 4],
    /// Countdown timer before game result is finalized (0 = no delay).
    pub round_end_timer: i32,
    /// Pending game result during the end-round delay.
    pub pending_result: Option<GameResult>,

    // ── Material palette cache ───────────────────────────────────────────────
    /// Material flags per palette index, derived from `tc.data.constants.materials`.
    /// Stored here to avoid repeated lookups through `tc`.
    pub tc_materials: Vec<Material>,

    // ── Visual effects (cosmetic only, not part of determinism checksum) ─────
    /// Screen flash intensity — mirrors C++ `Game::screenFlash`.
    /// Set by sobject creation; decrements each frame; applied as palette brightness.
    pub screen_flash:   i32,
    /// Per-viewport camera shake (fixed-point Q16.16) — mirrors C++ `Viewport::shake`.
    /// Set by sobjects within viewport; decays by 4000/frame; offsets camera during render.
    pub viewport_shake: [i32; 4],
}

impl Game {
    /// Create a new game from a loaded TC.
    ///
    /// Generates procedural terrain and places worms at valid spawn points.
    pub fn new(tc: Tc, num_worms: usize, seed: u32) -> Self {
        let mut level = Level::new(WIDTH, HEIGHT);
        let mut rand  = Mwc::new(seed);

        // Convert raw u8 material flags → typed Material values.
        let tc_materials: Vec<Material> = tc
            .data
            .constants
            .materials
            .iter()
            .map(|&b| Material(b))
            .collect();

        // Generate procedural terrain (mirrors C++ sim_start_game → generateFromSettings).
        levelgen::generate(&mut level, &tc, &mut rand, true);

        // Rebuild material cache after generation (pixels changed).
        // (tc_materials slice is per palette-index, not per pixel, so it stays valid.)

        // Create worms and place at valid spawn points.
        let worms: Vec<Worm> = (0..num_worms.min(MAX_WORMS))
            .map(|i| {
                let mut w = Worm::new(i, 100, 3);
                // Try to spawn at a valid position (16×16 free space).
                if let Some((sx, sy)) = levelgen::select_spawn(&level, &tc_materials, &mut rand, 16, 16) {
                    w.pos = FixedVec::new(
                        Fixed((sx + 8) << 16),
                        Fixed((sy + 8) << 16),
                    );
                } else {
                    // Fallback: centre of level.
                    w.pos = FixedVec::new(Fixed((WIDTH as i32 / 2) << 16), Fixed((HEIGHT as i32 / 2) << 16));
                }
                w
            })
            .collect();

        let mut game = Self {
            level,
            worms,
            wobjects:       Vec::new(),
            nobjects:       Vec::new(),
            sobjects:       Vec::new(),
            bonuses:        Vec::new(),
            floating_damages: Vec::new(),
            rand,
            cycles:         0,
            seed,
            max_bonuses:    4,
            worm_health:    100,
            worm_lives:     3,
            blood_pct:      100,
            loading_time_factor: 100,
            shadows_enabled: true,
            friendly_fire:  true,
            random_weapons: false,
            sound_events:   Vec::new(),
            mode:           GameMode::default(),
            koth_ticks:     [0; 4],
            bomb_timer:     0,
            dirt_scores:    [0; 4],
            tag_worm:       None,
            worm_checkpoint: [0; 4],
            worm_laps:      [0; 4],
            round_end_timer: 0,
            pending_result: None,
            tc_materials,
            tc,
            screen_flash:   0,
            viewport_shake: [0; 4],
        };

        // Set worm health and lives from configurable settings
        for w in game.worms.iter_mut() {
            w.health = game.worm_health;
            w.lives = game.worm_lives;
        }

        // Initialize weapon ammo from TC data
        for w in game.worms.iter_mut() {
            for slot in 0..5 {
                let weapon_idx = w.weapon_slots[slot];
                if weapon_idx < game.tc.weapons.len() {
                    let ammo = game.tc.weapons[weapon_idx].ammo;
                    // 0 = unlimited in TC notation
                    w.weapon_ammo[slot] = if ammo <= 0 { -1 } else { ammo };
                }
            }
        }

        game
    }

    /// Load terrain pixel data exported by `sim_export_level`.
    ///
    /// `pixels` is a flat row-major array of palette indices (u8), dimensions `width × height`.
    /// This gives the Rust sim real terrain so gravity-settle and material checks work correctly.
    pub fn load_level_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        self.level = Level::new(width, height);
        for (i, &p) in pixels.iter().enumerate() {
            let x = (i as u32 % width) as i32;
            let y = (i as u32 / width) as i32;
            self.level.set_pixel(x, y, p);
        }
    }

    /// Overwrite simulation state from a snapshot exported by `sim_export_state`.
    ///
    /// Used by the `replay-diff` harness to bypass the C++ level generator:
    /// after C++ runs `sim_start_game(seed)`, it exports the resulting state here
    /// so both sims begin from identical values.
    pub fn load_exported_state(&mut self, state: &ExportedState) {
        use crate::fixed::Fixed;
        use crate::fixed::FixedVec;

        self.rand.x  = state.rand_x;
        self.rand.c  = state.rand_c;
        self.cycles  = state.cycles;

        // Resize worms vec to match.
        let n = (state.num_worms as usize).min(MAX_WORMS);
        self.worms.resize_with(n, || Worm::new(0, 100, 5));

        for (i, ws) in state.worms[..n].iter().enumerate() {
            let w = &mut self.worms[i];
            w.index          = i;
            w.pos            = FixedVec::new(Fixed(ws.pos_x), Fixed(ws.pos_y));
            w.vel            = FixedVec::new(Fixed(ws.vel_x), Fixed(ws.vel_y));
            w.aiming_angle   = ws.aiming_angle;
            w.health         = ws.health;
            w.lives          = ws.lives;
            w.kills          = ws.kills;
            w.timer          = ws.timer;
            w.current_weapon = ws.current_weapon;
            w.killed_timer   = ws.killed_timer;
            w.alive          = ws.visible != 0;
            // Worm constructor sets ready=true; matches C++ `Worm::ready` initial state.
            w.ready          = true;
        }
    }

    /// Advance the simulation by one frame.
    ///
    /// Mirrors C++ `Game::processFrame()` field-update order:
    /// 1. Process existing bonuses (gravity, bounce, timer)
    /// 2. Increment `cycles`
    /// 3. Bonus spawn check (`rand(CBonusDropChance) == 0`)
    /// 4. Process worms
    pub fn step(&mut self, _inputs: &[u32]) {
        // Clear sound events from previous frame.
        self.sound_events.clear();

        // Decay visual effects (cosmetic only — not determinism-critical).
        if self.screen_flash > 0 { self.screen_flash -= 1; }
        for shake in &mut self.viewport_shake {
            if *shake > 0 { *shake = (*shake - 4000).max(0); } // 4000 = C++ constant
        }

        // Decrement round-end timer if active.
        if self.round_end_timer > 0 { self.round_end_timer -= 1; }

        // ── 1. Process existing bonuses ───────────────────────────────────────
        for i in 0..self.bonuses.len() {
            let expired = if let Some(bonus) = &mut self.bonuses[i] {
                Self::tick_bonus(bonus, &self.level, &self.tc_materials, &self.tc.data.constants)
            } else {
                continue;
            };
            if expired {
                // C++: when bonus timer hits 0 it calls sobjectTypes[bonusSObjects[frame]].create()
                // before freeing the bonus.  Mirror the rand consumption here.
                let (bx, by, frame) = {
                    let b = self.bonuses[i].as_ref().unwrap();
                    (b.x >> 16, b.y >> 16, b.frame) // ftoi
                };
                // C++ frees the bonus INSIDE sobjectTypes.create() (via the bonus-chain loop),
                // not before it.  Call expiry rand first; the chain inside will set this slot
                // to None.  The trailing set is a safety net.
                self.consume_bonus_expiry_rand(frame, bx, by);
                self.bonuses[i] = None;  // safety: ensure freed if chain somehow missed it
            }
        }

        // ── 2a. Process SObjects (animate frames, remove expired) ─────────────────
        self.sobjects.retain_mut(|s| {
            s.anim_delay_left -= 1;
            if s.anim_delay_left <= 0 {
                let stype = &self.tc.sobjects[s.type_idx];
                s.anim_delay_left = stype.anim_delay.max(1);
                s.cur_frame += 1;
            }
            let stype = &self.tc.sobjects[s.type_idx];
            s.cur_frame <= stype.num_frames
        });

        // ── 2b. Process WObjects then NObjects ────────────────────────────────────
        // Mirrors C++ processFrame order: bonuses → sobjects → wobjects → nobjects → ++cycles.
        let mut effective_inputs = _inputs.to_vec();

        // If round-end timer is active, suppress all worm inputs (freeze gameplay).
        if self.round_end_timer > 0 {
            effective_inputs.iter_mut().for_each(|inp| *inp = 0);
        }

        // Generate bot inputs for bot-controlled worms before wobjects step
        for (wi, worm) in self.worms.iter().enumerate() {
            if !worm.is_bot || !worm.alive { continue; }

            let difficulty = worm.bot_difficulty;

            // Find nearest alive enemy worm
            let my_pos = worm.pos;
            let mut nearest_enemy: Option<(usize, i32, i32)> = None;
            for (ei, enemy) in self.worms.iter().enumerate() {
                if ei == wi || !enemy.alive || enemy.health <= 0 { continue; }
                if matches!(self.mode, GameMode::TeamDeathmatch) && worm.team_id == enemy.team_id { continue; }
                let dx = enemy.pos.x.to_int() - my_pos.x.to_int();
                let dy = enemy.pos.y.to_int() - my_pos.y.to_int();
                nearest_enemy = Some((ei, dx, dy));
                break;
            }

            let mut bot_input: u32 = 0;
            if let Some((enemy_idx, dx, dy)) = nearest_enemy {
                use crate::worm::input;

                match difficulty {
                    0 => { // Easy — basic with edge avoidance
                        use crate::level::WIDTH;
                        let my_x = my_pos.x.to_int();
                        let edge_margin = 40;

                        // Avoid edges: if near left edge, move right; near right edge, move left
                        if my_x < edge_margin {
                            bot_input |= input::RIGHT;
                        } else if my_x > (WIDTH as i32 - edge_margin) {
                            bot_input |= input::LEFT;
                        } else if dx > 5 && self.cycles % 2 == 0 {
                            bot_input |= input::RIGHT;
                        } else if dx < -5 && self.cycles % 2 == 0 {
                            bot_input |= input::LEFT;
                        }
                        if dy < -20 { bot_input |= input::JUMP; }
                        let dist_sq = dx as i64 * dx as i64 + dy as i64 * dy as i64;
                        if dist_sq < 60 * 60 { bot_input |= input::FIRE; }
                        if self.cycles % 120 == 0 { bot_input |= input::JUMP; }
                    }
                    2 => { // Hard — predictive aim, rope tactics, dodge incoming projectiles
                        use crate::level::WIDTH;
                        let my_x = my_pos.x.to_int();
                        let edge_margin = 40;

                        // Avoid edges
                        if my_x < edge_margin {
                            bot_input |= input::RIGHT;
                        } else if my_x > (WIDTH as i32 - edge_margin) {
                            bot_input |= input::LEFT;
                        } else if dx > 5 {
                            bot_input |= input::RIGHT;
                        } else if dx < -5 {
                            bot_input |= input::LEFT;
                        }

                        if dy < -20 { bot_input |= input::JUMP; }

                        // Predictive aiming: add enemy velocity * 5 to target
                        let target_x = dx;
                        let target_y = dy;
                        if enemy_idx < self.worms.len() {
                            let enemy_vel_x = self.worms[enemy_idx].vel.x.to_int();
                            let enemy_vel_y = self.worms[enemy_idx].vel.y.to_int();
                            let pred_x = target_x + enemy_vel_x * 5;
                            let pred_y = target_y + enemy_vel_y * 5;
                            let dist_sq = pred_x as i64 * pred_x as i64 + pred_y as i64 * pred_y as i64;
                            if dist_sq < 150 * 150 { bot_input |= input::FIRE; }
                        } else {
                            let dist_sq = target_x as i64 * target_x as i64 + target_y as i64 * target_y as i64;
                            if dist_sq < 150 * 150 { bot_input |= input::FIRE; }
                        }

                        // Dodge incoming projectiles: check if any wobject is heading toward us
                        for wobj in &self.wobjects {
                            if wobj.owner_idx == wi { continue; }
                            let wobj_x = wobj.pos.x.to_int();
                            let wobj_y = wobj.pos.y.to_int();
                            let obj_dx = wobj_x - my_x;
                            let obj_dy = wobj_y - (my_pos.y.to_int());
                            let dist_sq = obj_dx as i64 * obj_dx as i64 + obj_dy as i64 * obj_dy as i64;
                            if dist_sq < 30 * 30 {
                                // Incoming! Jump to dodge
                                bot_input |= input::JUMP;
                                break;
                            }
                        }

                        // Rope to reach elevated enemies
                        if enemy_idx < self.worms.len() && dy < -30 {
                            bot_input |= input::ROPE;
                        }

                        if self.cycles % 50 == 0 { bot_input |= input::JUMP; }
                    }
                    _ => { // Medium (default, difficulty 1 or unknown) — weapon switching + stuck detection + edge avoidance
                        use crate::level::WIDTH;
                        let my_x = my_pos.x.to_int();
                        let edge_margin = 40;

                        // Avoid edges
                        if my_x < edge_margin {
                            bot_input |= input::RIGHT;
                        } else if my_x > (WIDTH as i32 - edge_margin) {
                            bot_input |= input::LEFT;
                        } else if dx > 5 {
                            bot_input |= input::RIGHT;
                        } else if dx < -5 {
                            bot_input |= input::LEFT;
                        }

                        if dy < -20 { bot_input |= input::JUMP; }
                        let dist_sq = dx as i64 * dx as i64 + dy as i64 * dy as i64;
                        if dist_sq < 100 * 100 { bot_input |= input::FIRE; }

                        // Weapon switching: if current weapon is reloading, switch to a ready weapon
                        let cur_slot = worm.current_weapon as usize;
                        if cur_slot < worm.reload_timers.len() && worm.reload_timers[cur_slot] > 0 {
                            for slot in 0..5 {
                                if slot < worm.reload_timers.len() && worm.reload_timers[slot] == 0 {
                                    bot_input |= input::CHANGE;
                                    break;
                                }
                            }
                        }

                        if self.cycles % 90 == 0 { bot_input |= input::JUMP; }
                    }
                }
            } else {
                // Wander: alternate left/right every 40 frames
                use crate::worm::input;
                if (self.cycles / 40) % 2 == 0 { bot_input |= input::RIGHT; }
                else { bot_input |= input::LEFT; }
                if self.cycles % 90 == 0 { bot_input |= input::JUMP; }
            }

            // Mode-specific bot behavior overrides
            let mode = self.mode.clone();
            use crate::worm::input;
            match &mode {
                GameMode::GameOfTag { .. } => {
                    // Prioritize chasing the "it" worm if we're not it
                    // Or run away if we ARE it (we don't want to be caught)
                    if let Some(tag_idx) = self.tag_worm {
                        if tag_idx != wi {
                            // Chase the "it" worm to tag them
                            let target = &self.worms[tag_idx];
                            let my_pos_x = self.worms[wi].pos.x.to_int();
                            let my_pos_y = self.worms[wi].pos.y.to_int();
                            let dx = target.pos.x.to_int() - my_pos_x;
                            let dy = target.pos.y.to_int() - my_pos_y;
                            bot_input = 0;
                            if dx > 5 { bot_input |= input::RIGHT; }
                            else if dx < -5 { bot_input |= input::LEFT; }
                            if dy < -20 { bot_input |= input::JUMP; }
                            let dist_sq = dx as i64 * dx as i64 + dy as i64 * dy as i64;
                            if dist_sq < 80 * 80 { bot_input |= input::FIRE; }
                        }
                    }
                }
                GameMode::Race { ref checkpoints, .. } => {
                    // Navigate toward the next checkpoint
                    let cp_idx = self.worm_checkpoint[wi] as usize;
                    if cp_idx < checkpoints.len() {
                        let (cpx, cpy) = checkpoints[cp_idx];
                        let my_x = self.worms[wi].pos.x.to_int();
                        let my_y = self.worms[wi].pos.y.to_int();
                        let dx = cpx - my_x;
                        let dy = cpy - my_y;
                        bot_input = 0; // Override: ignore enemies, go to checkpoint
                        if dx > 10 { bot_input |= input::RIGHT; }
                        else if dx < -10 { bot_input |= input::LEFT; }
                        if dy < -15 { bot_input |= input::JUMP; }
                        // Use rope to reach elevated checkpoints
                        if dy < -30 && self.cycles % 30 == 0 { bot_input |= input::ROPE; }
                    }
                }
                GameMode::KingOfHill { .. } => {
                    // Try to stay in the zone (level center area)
                    let zone_x = (WIDTH as i32) / 2;
                    let my_x = self.worms[wi].pos.x.to_int();
                    let dx_zone = zone_x - my_x;
                    // If far from zone, move toward it
                    if dx_zone.abs() > 50 {
                        if dx_zone > 0 { bot_input |= input::RIGHT; }
                        else { bot_input |= input::LEFT; }
                    }
                }
                _ => {} // Use default bot behavior for other modes
            }

            if wi < effective_inputs.len() {
                effective_inputs[wi] = bot_input;
            }
        }
        self.step_wobjects(&effective_inputs);
        self.step_nobjects();
        self.step_floating_damages();

        // ── 3. Increment frame counter ────────────────────────────────────────
        self.cycles = self.cycles.wrapping_add(1);

        // ── 4. Bonus spawn check (mirrors C++ rand(CBonusDropChance) == 0) ────
        let bonus_drop_chance = self.tc.data.constants.bonus_drop_chance;
        let bonus_disable     = self.tc.data.hacks.bonus_disable;
        if !bonus_disable
            && self.max_bonuses > 0
            && self.rand.rand(bonus_drop_chance as u32) == 0
        {
            self.create_bonus();
        }

        // ── 4. Process worms ──────────────────────────────────────────────────
        let num_worms  = self.worms.len();
        let cycles_now = self.cycles; // already incremented above
        for wi in 0..num_worms {
            let inp = if wi < _inputs.len() { _inputs[wi] } else { 0 };
            if self.worms[wi].alive {
                // ── Input: walk / aim / jump ─────────────────────────────────
                {
                    use crate::worm::input;
                    let consts = &self.tc.data.constants;
                    let w = &mut self.worms[wi];
                    let on_ground = w.reacts[react::UP] > 0;

                    // Walk left / right (Sprint Boots: +50% walk velocity).
                    // Direction change: mirror the aiming angle so the crosshair stays pointing
                    // in the same visual direction. Mirrors C++ `aimingAngle = 128 - aimingAngle`.
                    let sprint      = w.sprint_timer > 0;
                    let change_held = inp & input::CHANGE != 0;
                    let left        = inp & input::LEFT  != 0;
                    let right       = inp & input::RIGHT != 0;
                    // Direction flip only when not cycling weapons (mirrors C++ processMovement
                    // running independently of processWeaponChange).
                    if !change_held && left && !right && w.direction != 0 {
                        // Turning left: mirror angle if it's in right-facing range (>= 64).
                        let ai = (w.aiming_angle >> 16) & 0x7f;
                        if ai >= 64 {
                            w.aiming_angle = (128 - ai) << 16;
                        }
                        w.aim_vel = 0;
                        w.direction = 0;
                    }
                    if !change_held && right && !left && w.direction != 1 {
                        // Turning right: mirror angle if it's in left-facing range (<= 64).
                        let ai = (w.aiming_angle >> 16) & 0x7f;
                        if ai <= 64 {
                            w.aiming_angle = (128 - ai) << 16;
                        }
                        w.aim_vel = 0;
                        w.direction = 1;
                    }
                    // Walk velocity is always applied (C++ processMovement is independent of
                    // processWeaponChange). This allows swinging on the rope while reeling.
                    if left && !right {
                        let dv = if sprint { consts.walk_vel_left * 3 / 2 } else { consts.walk_vel_left };
                        w.vel.x.0 = (w.vel.x.0 - dv).max(consts.max_vel_left);
                    }
                    if right && !left {
                        let dv = if sprint { consts.walk_vel_right * 3 / 2 } else { consts.walk_vel_right };
                        w.vel.x.0 = (w.vel.x.0 + dv).min(consts.max_vel_right);
                    }

                    // Jump — rising-edge only (C++ uses pressedOnce / ableToJump semantics).
                    // Holding JUMP on ground would otherwise chain-jump every frame.
                    let jump_just = inp & input::JUMP != 0 && w.prev_inp & input::JUMP == 0;
                    let can_jump  = on_ground || (w.extra_jumps > w.jumps_used);
                    if jump_just && can_jump {
                        if !on_ground { w.jumps_used += 1; }
                        w.vel.y.0 -= consts.jump_force;
                    }
                    if on_ground { w.jumps_used = 0; }

                    // Aim (smooth angular velocity).
                    // C++ condition (worm.cpp processAiming): allow aim unless rope is deployed
                    // AND Change is held (Change+UP/DOWN = reel in/out instead of aiming).
                    let can_aim = !w.rope_active || !change_held;
                    if can_aim {
                        if inp & input::UP != 0 {
                            w.aim_vel = (w.aim_vel + consts.aim_acc_left).min(consts.max_aim_vel_left);
                        } else if inp & input::DOWN != 0 {
                            w.aim_vel = (w.aim_vel - consts.aim_acc_right).max(consts.max_aim_vel_right);
                        } else {
                            w.aim_vel = (w.aim_vel as i64 * consts.aim_fric_mult as i64
                                / consts.aim_fric_div as i64) as i32;
                        }
                        // Apply aim velocity (angle wraps 0-127).
                        // Invert delta for right-facing worms so UP always moves crosshair upward:
                        // right-facing arc is [64,116] where 64=up; increasing angle aims downward.
                        let delta = if w.direction != 0 { -w.aim_vel } else { w.aim_vel };
                        w.aiming_angle = w.aiming_angle.wrapping_add(delta);
                        // Clamp to firing arc based on direction.
                        let angle_int = ((w.aiming_angle >> 16) & 0x7f) as i32;
                        let (lo, hi) = if w.direction != 0 {
                            (consts.aim_min_right, consts.aim_max_right) // facing right
                        } else {
                            (consts.aim_max_left, consts.aim_min_left)   // facing left
                        };
                        let clamped = angle_int.clamp(lo, hi);
                        if clamped != angle_int {
                            w.aiming_angle = clamped << 16;
                            w.aim_vel = 0;
                        }
                    }

                    // Tick power-up timers.
                    if w.sprint_timer > 0 { w.sprint_timer -= 1; }
                    if w.kevlar_timer > 0 { w.kevlar_timer -= 1; }
                    if w.damage_flash > 0 { w.damage_flash -= 1; }

                    // Decrement shell casing timer and spawn when it reaches zero.
                    if w.leave_shell_timer > 0 {
                        w.leave_shell_timer -= 1;
                        if w.leave_shell_timer == 0 {
                            // Spawn shell at current aiming angle
                            let angle = (w.aiming_angle >> 16) as usize & 0x7f;
                            self.spawn_shell_casing(wi, angle);
                        }
                    }
                }

                // Dig: LEFT+RIGHT simultaneously digs toward the aim direction.
                // Mirrors C++ worm.cpp: left&&right → drawDirtEffect(7, pos+dir*2-7, pos+dir*4-7).
                {
                    use crate::worm::input;
                    use crate::math::COSSIN_TABLE;
                    let left  = inp & input::LEFT  != 0;
                    let right = inp & input::RIGHT != 0;
                    if left && right {
                        let do_dig = self.worms[wi].able_to_dig;
                        self.worms[wi].able_to_dig = false;
                        if do_dig {
                            let (px, py, angle) = {
                                let w = &self.worms[wi];
                                (w.pos.x.to_int(), w.pos.y.to_int(),
                                 (w.aiming_angle >> 16) as usize & 0x7f)
                            };
                            let (dx, dy) = COSSIN_TABLE[angle]; // Q16.16 unit direction
                            // Two dig circles at dir*2 and dir*4 from worm center.
                            // apply_dirt_effect expects top-left of 16×16 sprite, so subtract 7.
                            for dist in [2i32, 4i32] {
                                let cx = px + (dx * dist) / 65536 - 7;
                                let cy = py + (dy * dist) / 65536 - 7;
                                let rframe = {
                                    let tex = self.tc.data.constants.textures.get(7);
                                    tex.map(|t| t.rframe as u32).unwrap_or(1)
                                };
                                let rand_result = self.rand.rand(rframe);
                                self.apply_dirt_effect(7, cx, cy, rand_result);
                            }
                        }
                    } else {
                        self.worms[wi].able_to_dig = true;
                    }
                }

                // ── Ninja rope: fire / retract / update flying hook ───────────────────
                // Matches original Liero ninjarope.cpp + worm.cpp logic.
                // Rope is fired with Change+Jump or the dedicated ROPE key.
                // Physics: spring force toward hook (not position constraint).
                {
                    use crate::worm::input;
                    use crate::math::COSSIN_TABLE;
                    use crate::fixed::Fixed;

                    let change_held       = inp & input::CHANGE != 0;
                    let jump_held         = inp  & input::JUMP != 0;
                    let jump_just         = jump_held && (self.worms[wi].prev_inp & input::JUMP == 0);
                    let rope_just_pressed = (inp  & input::ROPE != 0) && (self.worms[wi].prev_inp & input::ROPE == 0);
                    // C++ worm.cpp: Change+Jump → fire/toggle rope; Jump alone → retract rope.
                    let fire_rope    = rope_just_pressed || (change_held && jump_just);
                    let retract_rope = !change_held && jump_held; // Jump alone continuously retracts

                    let rope_active   = self.worms[wi].rope_active;
                    let rope_attached = self.worms[wi].rope_attached;
                    let consts        = &self.tc.data.constants;

                    // Jump alone while rope is out: retract immediately.
                    if retract_rope && rope_active && !fire_rope {
                        self.worms[wi].rope_active   = false;
                        self.worms[wi].rope_attached = false;
                    }

                    if fire_rope {
                        if rope_active {
                            // Retract (Change+Jump while rope is out).
                            self.worms[wi].rope_active   = false;
                            self.worms[wi].rope_attached = false;
                        } else {
                            let (pos_x, pos_y, angle) = {
                                let w = &self.worms[wi];
                                (w.pos.x, w.pos.y, (w.aiming_angle >> 16) as usize & 0x7f)
                            };
                            let (cs_dx, cs_dy) = COSSIN_TABLE[angle];
                            let nr_throw_x = consts.nr_throw_vel_x;
                            let nr_throw_y = consts.nr_throw_vel_y;
                            let nr_init    = consts.nr_initial_length;
                            let w = &mut self.worms[wi];
                            w.rope_active     = true;
                            w.rope_attached   = false;
                            w.rope_hook.x     = pos_x;
                            w.rope_hook.y     = pos_y;
                            // Velocity: cossin << NRThrowVelX (original: vel = cossin << 2)
                            w.rope_hook_vel.x = Fixed(cs_dx << nr_throw_x);
                            w.rope_hook_vel.y = Fixed(cs_dy << nr_throw_y);
                            w.rope_len        = nr_init;
                            self.sound_events.push(SoundEvent::RopeThrown { worm_idx: wi });
                        }
                    }

                    // Move flying hook, apply hook gravity, check terrain.
                    if rope_active && !rope_attached {
                        let nr_gravity = consts.ninjrarope_gravity;
                        let nr_attach  = consts.nr_attach_length;
                        let _nr_min     = consts.nr_min_length;
                        let _nr_max     = consts.nr_max_length;
                        let shl_x      = consts.nr_force_shl_x;
                        let div_x      = consts.nr_force_div_x;
                        let shl_y      = consts.nr_force_shl_y;
                        let div_y      = consts.nr_force_div_y;
                        let shl_len    = consts.nr_force_len_shl;

                        // Hook gravity + movement
                        {
                            let w = &mut self.worms[wi];
                            w.rope_hook_vel.y.0 += nr_gravity; // gravity on flying hook
                            w.rope_hook.x.0 += w.rope_hook_vel.x.0;
                            w.rope_hook.y.0 += w.rope_hook_vel.y.0;
                        }

                        let (hx, hy, worm_x, worm_y, rope_len) = {
                            let w = &self.worms[wi];
                            (w.rope_hook.x.to_int(), w.rope_hook.y.to_int(),
                             w.pos.x.0, w.pos.y.0, w.rope_len)
                        };

                        if self.level.inside(hx, hy) {
                            let mat = self.level.material(hx, hy, &self.tc_materials);
                            if !mat.background() {
                                // Attached to terrain — spawn 11 dirt particles (ninjarope.cpp).
                                self.worms[wi].rope_attached = true;
                                self.worms[wi].rope_hook_vel.x.0 = 0;
                                self.worms[wi].rope_hook_vel.y.0 = 0;
                                self.worms[wi].rope_len = nr_attach; // NRAttachLength = 450
                                // Dirt spray at impact point (nobject type index 2 = "dirt").
                                let hook_pos = self.worms[wi].rope_hook;
                                let dirt_idx = self.tc.data.types.nobjects.iter().position(|n| n == "dirt").unwrap_or(2);
                                for _ in 0..11 {
                                    let angle = self.rand.rand(128) as usize;
                                    self.create_nobject2(dirt_idx, angle, FixedVec::ZERO, hook_pos);
                                }
                            } else {
                                // Flying — apply rope spring force back toward worm if over-extended
                                let diff_x = self.worms[wi].rope_hook.x.0 - worm_x;
                                let diff_y = self.worms[wi].rope_hook.y.0 - worm_y;
                                let dpx = diff_x >> 16;
                                let dpy = diff_y >> 16;
                                let dist = isqrt32(dpx * dpx + dpy * dpy);
                                let cur_len = (dist + 1) << shl_len;
                                if cur_len > rope_len {
                                    let fx = (diff_x << shl_x) / div_x;
                                    let fy = (diff_y << shl_y) / div_y;
                                    self.worms[wi].rope_hook_vel.x.0 -= fx / cur_len;
                                    self.worms[wi].rope_hook_vel.y.0 -= fy / cur_len;
                                }
                            }
                        } else {
                            // Hook flew off map
                            self.worms[wi].rope_active = false;
                        }
                    }
                }

                // Alive: run terrain physics.
                {
                    let vel_before = self.worms[wi].vel;
                    Self::step_worm_physics(
                        &mut self.worms[wi],
                        &self.level,
                        &self.tc_materials,
                        &self.tc.data.constants,
                        self.tc.data.hacks.fall_damage,
                    );
                    let vel_after = self.worms[wi].vel;
                    // Detect bounce: if velocity was negated (bounce happened) and is significant.
                    if vel_before.y.0.abs() > 5000 && vel_after.y.0 * vel_before.y.0 < 0 {
                        self.sound_events.push(SoundEvent::WormBounce { worm_idx: wi });
                    }
                }

                // ── Rope spring force (applied after physics, matching ninjarope.cpp) ──
                if self.worms[wi].rope_active && self.worms[wi].rope_attached {
                    let (hook_x, hook_y, worm_x, worm_y, rope_len) = {
                        let w = &self.worms[wi];
                        (w.rope_hook.x.0, w.rope_hook.y.0,
                         w.pos.x.0, w.pos.y.0,
                         w.rope_len)
                    };
                    let consts  = &self.tc.data.constants;
                    let shl_x   = consts.nr_force_shl_x;
                    let div_x   = consts.nr_force_div_x;
                    let shl_y   = consts.nr_force_shl_y;
                    let div_y   = consts.nr_force_div_y;
                    let shl_len = consts.nr_force_len_shl;

                    // diff = hook - worm (direction toward hook), Q16.16
                    let diff_x = hook_x - worm_x;
                    let diff_y = hook_y - worm_y;
                    let dpx = diff_x >> 16; // integer pixels
                    let dpy = diff_y >> 16;
                    let dist    = isqrt32(dpx * dpx + dpy * dpy);
                    let cur_len = (dist + 1) << shl_len; // TC units

                    // Apply spring force when rope is over-extended
                    if cur_len > rope_len {
                        let force_x = (diff_x << shl_x) / div_x;
                        let force_y = (diff_y << shl_y) / div_y;
                        self.worms[wi].vel.x.0 += force_x / cur_len;
                        self.worms[wi].vel.y.0 += force_y / cur_len;
                    }

                    // Reel: Change+Up = pull in, Change+Down = let out.
                    use crate::worm::input as ib;
                    let change_held = inp & ib::CHANGE != 0;
                    if change_held {
                        let nr_pull    = consts.nr_pull_vel;
                        let nr_release = consts.nr_release_vel;
                        let nr_min     = consts.nr_min_length;
                        let nr_max     = consts.nr_max_length;
                        if inp & ib::UP != 0 {
                            self.worms[wi].rope_len = (self.worms[wi].rope_len - nr_pull).max(nr_min);
                        } else if inp & ib::DOWN != 0 {
                            self.worms[wi].rope_len = (self.worms[wi].rope_len + nr_release).min(nr_max);
                        }
                    }
                }

                // ── Worm-to-worm collision (Sprint-38) ────────────────────────────────────
                // Push apart colliding worms after terrain physics is resolved.
                // Only process worms i > current worm index to avoid duplicate pairs.
                const COLLISION_RADIUS: i32 = 7;
                const COLLISION_RADIUS_SQ: i32 = COLLISION_RADIUS * COLLISION_RADIUS;
                {
                    for wj in (wi + 1)..self.worms.len() {
                        let wj_worm = &self.worms[wj];
                        // Only push if both worms are alive with health > 0.
                        if !wj_worm.alive || wj_worm.health <= 0 {
                            continue;
                        }

                        let xi = self.worms[wi].pos.x.to_int();
                        let yi = self.worms[wi].pos.y.to_int();
                        let xj = wj_worm.pos.x.to_int();
                        let yj = wj_worm.pos.y.to_int();
                        let dx = xi - xj;
                        let dy = yi - yj;

                        if dx * dx + dy * dy < COLLISION_RADIUS_SQ {
                            // Worms are colliding — compute push direction and magnitude.
                            let dist_sq = (dx * dx + dy * dy).max(1); // avoid division by zero
                            let dist = isqrt32(dist_sq);

                            // Push force: (COLLISION_RADIUS - distance) * 0.5 in fixed-point units.
                            // Using Fixed(500) as a small fixed-point push constant.
                            let push_force = ((COLLISION_RADIUS - dist) * 500 / 2).max(1);

                            // Normalize direction (dx, dy) → (dir_x, dir_y).
                            // Direction is from j to i, so we push i away and j the opposite way.
                            // To avoid floating-point, we scale: push_vel = push_force * (dx, dy) / dist
                            let push_vel_x = (dx as i64 * push_force as i64 / dist as i64) as i32;
                            let push_vel_y = (dy as i64 * push_force as i64 / dist as i64) as i32;

                            // Apply push to both worms (capped to avoid explosions).
                            const MAX_PUSH: i32 = 100_000; // ~1.5 pixels/frame in fixed-point
                            self.worms[wi].vel.x.0 += push_vel_x.clamp(-MAX_PUSH, MAX_PUSH);
                            self.worms[wi].vel.y.0 += push_vel_y.clamp(-MAX_PUSH, MAX_PUSH);
                            self.worms[wj].vel.x.0 -= push_vel_x.clamp(-MAX_PUSH, MAX_PUSH);
                            self.worms[wj].vel.y.0 -= push_vel_y.clamp(-MAX_PUSH, MAX_PUSH);
                        }
                    }
                }

                // ── 4.1. Bonus collection — mirrors C++ Worm::process() bonus-proximity loop.
                //
                // For each bonus within 5 integer pixels of the worm:
                //   weapon (frame==0): consume rand(BonusExplodeRisk); remove bonus;
                //                      if result ≤ 1: consume sobjectTypes[0] (large_explosion) rand.
                //   health (frame==1): if worm health < max (100): consume rand(BonusHealthVar);
                //                      apply healing; remove bonus.
                {
                    let wx = self.worms[wi].pos.x.to_int();
                    let wy = self.worms[wi].pos.y.to_int();
                    let w_health = self.worms[wi].health;
                    let max_health = 100i32; // C++ default WormSettings::health

                    let mut bi = 0;
                    while bi < self.bonuses.len() {
                        if let Some(bonus) = &self.bonuses[bi] {
                            let bx = bonus.x >> 16; // ftoi
                            let by = bonus.y >> 16;
                            if (wx - bx).abs() < 5 && (wy - by).abs() < 5 {
                                let frame = bonus.frame;
                                if frame == 0 {
                                    // Weapon bonus — always consume rand(BonusExplodeRisk).
                                    let explode_risk = self.tc.data.constants.bonus_explode_risk as u32;
                                    let result = self.rand.rand(explode_risk);
                                    self.bonuses[bi] = None;
                                    if result <= 1 {
                                        // Bonus explodes → sobjectTypes[0] = large_explosion.
                                        self.consume_sobject_rand_by_idx(0, bx, by);
                                    }
                                    // Don't advance bi
                                } else if frame == 1 && w_health < max_health {
                                    // Health bonus — only collected when not at full health.
                                    let health_var = self.tc.data.constants.bonus_health_var as u32;
                                    let health_min = self.tc.data.constants.bonus_min_health;
                                    let heal = self.rand.rand(health_var) as i32 + health_min;
                                    self.bonuses[bi] = None;
                                    self.worms[wi].health = (w_health + heal).min(max_health);
                                    self.sound_events.push(SoundEvent::BonusCollect);
                                } else if frame == 2 {
                                    // Sprint Boots — 30 s of 50% speed boost.
                                    self.bonuses[bi] = None;
                                    self.worms[wi].sprint_timer = 1800;
                                    self.sound_events.push(SoundEvent::BonusCollect);
                                } else if frame == 3 {
                                    // Kevlar — 15 s of 50% damage reduction.
                                    self.bonuses[bi] = None;
                                    self.worms[wi].kevlar_timer = 900;
                                    self.sound_events.push(SoundEvent::BonusCollect);
                                } else if frame == 4 {
                                    // Double Jump — grants one extra mid-air jump.
                                    self.bonuses[bi] = None;
                                    self.worms[wi].extra_jumps = 1;
                                    self.sound_events.push(SoundEvent::BonusCollect);
                                } else {
                                    bi += 1;
                                }
                            } else {
                                bi += 1;
                            }
                        } else {
                            bi += 1;
                        }
                    }
                }
                // ── Weapon processing: reload timers + firing ────────────────────────
                {
                    use crate::worm::input;

                    // Decrement all per-slot reload timers.
                    for (_slot, rt) in self.worms[wi].reload_timers.iter_mut().enumerate() {
                        if *rt > 0 {
                            *rt -= 1;
                            if *rt == 0 {
                                self.sound_events.push(SoundEvent::WeaponReloaded { worm_idx: wi });
                            }
                        }
                    }

                    // Decrement fire_cone (rapid-fire inter-shot cooldown).
                    if self.worms[wi].fire_cone > 0 { self.worms[wi].fire_cone -= 1; }

                    let cur_slot   = self.worms[wi].current_weapon as usize;
                    let cur_weapon = self.worms[wi].weapon_slots.get(cur_slot).copied().unwrap_or(cur_slot);
                    let has_ammo   = self.worms[wi].weapon_ammo.get(cur_slot).copied().unwrap_or(-1) != 0;
                    let can_fire   = self.worms[wi].reload_timers.get(cur_slot).copied().unwrap_or(0) <= 0
                        && self.worms[wi].fire_cone == 0
                        && has_ammo;
                    let firing_now = inp & input::FIRE != 0;

                    if let Some(weapon) = self.tc.weapons.get(cur_weapon).cloned() {
                        if weapon.charge_stages > 0 && can_fire {
                            // ── Charge-up weapon (Gauss Sniper) ─────────────────
                            if firing_now {
                                // Accumulate charge while FIRE held.
                                self.worms[wi].charge_ticks += 1;
                            } else if self.worms[wi].prev_firing {
                                // FIRE just released → compute charge stage and fire.
                                let ticks = self.worms[wi].charge_ticks;
                                let stage = if weapon.charge_time > 0 {
                                    (ticks / weapon.charge_time).min(weapon.charge_stages)
                                } else {
                                    weapon.charge_stages
                                };
                                self.worms[wi].charge_ticks = 0;

                                // Scale: stage 0 = 25% power, full stage = 100%.
                                let scale_pct = 25 + 75 * stage / weapon.charge_stages;

                                let angle    = (self.worms[wi].aiming_angle >> 16) as usize & 0x7f;
                                let worm_vel = self.worms[wi].vel;
                                let slot     = cur_slot.min(4);
                                let effective_reload = weapon.loading_time * self.loading_time_factor / 100;
                                self.worms[wi].reload_timers[slot] = effective_reload;

                                // Decrement ammo
                                if self.worms[wi].weapon_ammo[slot] > 0 {
                                    self.worms[wi].weapon_ammo[slot] -= 1;
                                }
                                // -1 means unlimited, don't decrement

                                let recoil = weapon.recoil * scale_pct / 100;
                                if recoil != 0 {
                                    let (cx, cy) = COSSIN_TABLE[angle];
                                    self.worms[wi].vel.x.0 -= ((cx as i64 * recoil as i64) / 100) as i32;
                                    self.worms[wi].vel.y.0 -= ((cy as i64 * recoil as i64) / 100) as i32;
                                }

                                self.sound_events.push(SoundEvent::WeaponFire { weapon_idx: cur_weapon });
                                self.worms[wi].fire_cone = weapon.fire_cone;
                                self.create_wobject_charged(cur_weapon, wi, angle, worm_vel, scale_pct);
                                // Schedule shell casing spawn with delay and probability
                                if weapon.leave_shells > 0 && self.rand.rand(weapon.leave_shells as u32) == 0 {
                                    self.worms[wi].leave_shell_timer = weapon.leave_shell_delay.max(1);
                                }
                            }
                        } else if firing_now && can_fire {
                            // ── Normal instant-fire weapon ───────────────────────
                            let loading_time = weapon.loading_time * self.loading_time_factor / 100;
                            let angle        = (self.worms[wi].aiming_angle >> 16) as usize & 0x7f;
                            let worm_vel     = self.worms[wi].vel;
                            let parts        = weapon.parts.max(1);
                            let distribution = weapon.distribution;

                            let slot = cur_slot.min(4);
                            self.worms[wi].reload_timers[slot] = loading_time;

                            // Decrement ammo
                            if self.worms[wi].weapon_ammo[slot] > 0 {
                                self.worms[wi].weapon_ammo[slot] -= 1;
                            }
                            // -1 means unlimited, don't decrement

                            let recoil = weapon.recoil;
                            if recoil != 0 {
                                let (cx, cy) = COSSIN_TABLE[angle];
                                self.worms[wi].vel.x.0 -= ((cx as i64 * recoil as i64) / 100) as i32;
                                self.worms[wi].vel.y.0 -= ((cy as i64 * recoil as i64) / 100) as i32;
                            }

                            self.sound_events.push(SoundEvent::WeaponFire { weapon_idx: cur_weapon });
                            self.worms[wi].fire_cone = weapon.fire_cone;

                            for _ in 0..parts {
                                let fire_angle = if distribution > 0 {
                                    let spread = self.rand.rand((distribution * 2) as u32) as i32 - distribution;
                                    ((angle as i32 + spread).rem_euclid(128)) as usize
                                } else {
                                    angle
                                };
                                self.create_wobject(cur_weapon, wi, fire_angle, worm_vel);
                            }
                            // Schedule shell casing spawn with delay and probability
                            if weapon.leave_shells > 0 && self.rand.rand(weapon.leave_shells as u32) == 0 {
                                self.worms[wi].leave_shell_timer = weapon.leave_shell_delay.max(1);
                            }
                        }
                    }
                    // Reset charge if weapon switched.
                    if !firing_now && !self.worms[wi].prev_firing {
                        let prev_slot = cur_weapon;
                        let _ = prev_slot; // charge_ticks reset on release (handled above)
                    }
                    self.worms[wi].prev_firing = firing_now;
                }

                // ── Weapon change (CHANGE + LEFT/RIGHT, edge-triggered) ───────────────
                // Mirrors C++ `processWeaponChange`: CHANGE held → LEFT/RIGHT cycle slots.
                // `pressedOnce` semantics: only triggers on rising edge (key just pressed).
                {
                    use crate::worm::input;
                    const NUM_SLOTS: i32 = 5;
                    let w = &mut self.worms[wi];
                    if inp & input::CHANGE != 0 {
                        let prev = w.prev_inp;
                        // Rising edge: bit was 0 last frame, 1 this frame.
                        let just_left  = inp & input::LEFT  != 0 && prev & input::LEFT  == 0;
                        let just_right = inp & input::RIGHT != 0 && prev & input::RIGHT == 0;
                        if just_left {
                            w.current_weapon = (w.current_weapon - 1).rem_euclid(NUM_SLOTS);
                            w.charge_ticks = 0;
                        }
                        if just_right {
                            w.current_weapon = (w.current_weapon + 1) % NUM_SLOTS;
                            w.charge_ticks = 0;
                        }
                    }
                    w.prev_inp = inp;
                }

                // ── Rendering state (not part of checksum) ───────────────────────────
                {
                    use crate::worm::input;
                    let w = &mut self.worms[wi];
                    // Only update direction when exactly one of LEFT/RIGHT is pressed
                    // and CHANGE is not held (weapon cycling shouldn't turn the worm).
                    let change_held = inp & input::CHANGE != 0;
                    let left_only  = !change_held && inp & input::LEFT  != 0 && inp & input::RIGHT == 0;
                    let right_only = !change_held && inp & input::RIGHT != 0 && inp & input::LEFT  == 0;
                    if left_only  { w.direction = 0; }
                    if right_only { w.direction = 1; }

                    let on_ground = w.reacts[react::UP] > 0;
                    let moving    = !change_held && inp & (input::LEFT | input::RIGHT) != 0;
                    w.animate     = on_ground && moving;

                    let aiming = (w.aiming_angle >> 16) & 0x7f; // integer angle 0-127
                    let dir    = w.direction;
                    let angle_frame = {
                        let mut x = aiming - 12;
                        if dir != 0 { x -= 49; }
                        x >>= 3;
                        x = x.clamp(0, 6);
                        if dir != 0 { x = 6 - x; }
                        x
                    };
                    const WALK_TAB: [i32; 4] = [0, 7, 0, 14];
                    let walk_offset = if w.animate {
                        WALK_TAB[((cycles_now & 31) >> 3) as usize]
                    } else { 0 };
                    w.cur_frame = angle_frame + walk_offset;
                    w.visible   = true;
                }

                // ── Low-health bleeding (mirrors C++ worm.cpp:388-403) ─────────────
                // Spawn blood particles when worm health is below 25% (max_health / 4).
                {
                    let max_health = 100i32; // C++ default WormSettings::health
                    let w = &self.worms[wi];
                    if w.health > 0 && w.health < max_health / 4 {
                        // Use timer modulo to spawn periodically (every ~6 frames on average).
                        if w.timer % 6 == 0 {
                            let blood_idx = self.tc.data.types.nobjects.iter()
                                .position(|n| n == "blood").unwrap_or(6);
                            let angle = self.rand.rand(128) as usize;
                            let vel_in = FixedVec {
                                x: Fixed((self.rand.rand(100) as i32) - 50), // small random velocity
                                y: Fixed((self.rand.rand(100) as i32) - 50)
                            };
                            let pos = self.worms[wi].pos;
                            self.create_nobject2(blood_idx, angle, vel_in, pos);
                        }
                    }
                }
            } else {
                // Dead: respawn state machine mirrors C++ Worm::process() dead branch.
                self.worms[wi].rope_active   = false;
                self.worms[wi].rope_attached = false;
                self.worms[wi].visible = false;
                if self.worms[wi].killed_timer > 0 {
                    self.worms[wi].killed_timer -= 1;
                }

                if self.worms[wi].killed_timer == 0 {
                    // Decide: respawn or permanently eliminate.
                    let can_respawn = self.can_respawn(wi);
                    if can_respawn {
                        // ZombieMode: set zombie HP on respawn entry.
                        if matches!(self.mode, GameMode::ZombieMode)
                            && self.worms[wi].is_zombie
                        {
                            self.worms[wi].health = 50;
                        }
                        let (enemy_x, enemy_y) = Self::enemy_centroid(&self.worms, wi);
                        let old_x = self.worms[wi].pos.x.to_int();
                        let old_y = self.worms[wi].pos.y.to_int();
                        Self::begin_respawn(
                            &mut self.worms[wi],
                            &self.level,
                            &self.tc_materials,
                            &self.tc.data.constants,
                            &mut self.rand,
                            enemy_x, enemy_y,
                            old_x, old_y,
                        );
                        self.sound_events.push(SoundEvent::WormRespawn { worm_idx: wi });
                    } else {
                        // Permanently dead — park timer so this branch never fires again.
                        self.worms[wi].eliminated = true;
                        self.worms[wi].killed_timer = i32::MIN / 2;
                    }
                }

                // Only animate respawn when in the -1 state (set by begin_respawn).
                if self.worms[wi].killed_timer == -1 {
                    let tex0_rframe = self.tc.data.constants.textures
                        .get(0).map(|t| t.rframe as u32).unwrap_or(2);
                    Self::do_respawning(
                        &mut self.worms[wi],
                        &self.level,
                        &mut self.rand,
                        tex0_rframe,
                    );
                }
            }
        }

        // ── 5. Mode-specific per-frame effects ───────────────────────────────
        self.step_mode_effects();
    }

    /// Returns whether worm `wi` is allowed to respawn in the current game mode.
    fn can_respawn(&self, wi: usize) -> bool {
        match &self.mode {
            // These modes never permanently eliminate worms (no lives limit).
            GameMode::KingOfHill { .. } => true,
            GameMode::BombTag { .. }    => true,
            // Zombies always respawn; live humans who die become zombies (lives set to 999).
            GameMode::ZombieMode        => true,
            // Kill Race: infinite respawns.
            GameMode::KillRace { .. }   => true,
            // Race mode: infinite respawns so worms can continue through checkpoints.
            GameMode::Race { .. }       => true,
            // All other modes: respawn only while lives remain.
            _ => self.worms[wi].lives > 0,
        }
    }

    /// Per-frame game-mode side-effects: KotH zone ticks, BombTag fuse countdown, Race checkpoints.
    fn step_mode_effects(&mut self) {
        match self.mode.clone() {
            GameMode::KingOfHill { .. } => {
                // Zone: centre of level ± 40 px.
                use crate::level::{WIDTH, HEIGHT};
                let zx1 = WIDTH  as i32 / 2 - 40;
                let zx2 = WIDTH  as i32 / 2 + 40;
                let zy1 = HEIGHT as i32 / 2 - 40;
                let zy2 = HEIGHT as i32 / 2 + 40;
                for wi in 0..self.worms.len().min(4) {
                    if self.worms[wi].alive {
                        let px = self.worms[wi].pos.x.to_int();
                        let py = self.worms[wi].pos.y.to_int();
                        if px >= zx1 && px <= zx2 && py >= zy1 && py <= zy2 {
                            self.koth_ticks[wi] += 1;
                        }
                    }
                }
            }
            GameMode::BombTag { fuse } => {
                if self.bomb_timer > 0 {
                    self.bomb_timer -= 1;
                    if self.bomb_timer <= 0 {
                        // Bomb explodes on carrier.
                        if let Some(carrier) = self.worms.iter().position(|w| w.has_bomb) {
                            self.worms[carrier].has_bomb = false;
                            // Direct kill (skips the hitDamage path, mirrors C++ explosion).
                            self.do_damage(carrier, 9999, -1);
                        }
                        // Transfer bomb to a random alive worm and reset fuse.
                        if let Some(next) = self.worms.iter().position(|w| w.alive) {
                            self.worms[next].has_bomb = true;
                            self.bomb_timer = fuse;
                        }
                    }
                }
            }
            GameMode::Race { ref checkpoints, laps } => {
                for wi in 0..self.worms.len().min(4) {
                    if !self.worms[wi].alive { continue; }
                    let cp_idx = self.worm_checkpoint[wi] as usize;
                    if cp_idx >= checkpoints.len() { continue; }
                    let (cpx, cpy) = checkpoints[cp_idx];
                    let wx = self.worms[wi].pos.x.to_int();
                    let wy = self.worms[wi].pos.y.to_int();
                    let dx = wx - cpx;
                    let dy = wy - cpy;
                    if dx * dx + dy * dy < 15 * 15 {
                        // Reached checkpoint
                        self.worm_checkpoint[wi] += 1;
                        if self.worm_checkpoint[wi] as usize >= checkpoints.len() {
                            // Completed a lap
                            self.worm_laps[wi] += 1;
                            self.worm_checkpoint[wi] = 0;
                            if self.worm_laps[wi] >= laps {
                                // This worm wins!
                                self.pending_result = Some(GameResult::WormWins(wi));
                                self.round_end_timer = 180;
                            }
                        }
                    }
                }
            }
            GameMode::GameOfTag { time_limit } => {
                if let Some(tag_idx) = self.tag_worm {
                    if tag_idx < self.worms.len() {
                        let worm = &mut self.worms[tag_idx];
                        if worm.alive && worm.health > 0 {
                            worm.timer += 1;
                            if worm.timer >= time_limit {
                                self.pending_result = Some(GameResult::WormWins(tag_idx));
                                self.round_end_timer = 180;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ── Bonus helpers ─────────────────────────────────────────────────────

    /// Port of `Bonus::process()` in `src/game/bonus.cpp`.
    ///
    /// Returns `true` if the bonus has expired (timer hit 0).
    /// Caller is responsible for removing the bonus and consuming the sobject creation rand.
    fn tick_bonus(
        bonus:   &mut Bonus,
        level:   &Level,
        tc_mat:  &[Material],
        consts:  &liero_data::tc::TcConstants,
    ) -> bool {
        // Step position.
        bonus.y += bonus.vel_y;

        let ix = bonus.x >> 16; // ftoi
        let iy = bonus.y >> 16; // ftoi

        // Apply gravity while below pixel is background.
        if level.inside(ix, iy + 1) && level.material(ix, iy + 1, tc_mat).background() {
            bonus.vel_y += consts.bonus_gravity;
        }

        // Bounce if next-frame position would hit terrain or OOB.
        let inew_y = (bonus.y + bonus.vel_y) >> 16;
        if inew_y < 0
            || inew_y >= level.height as i32 - 1
            || level.material(ix, inew_y, tc_mat).dirt_rock()
        {
            bonus.vel_y = -(bonus.vel_y * consts.bonus_bounce_mul) / consts.bonus_bounce_div;
            if bonus.vel_y.abs() < 100 {
                bonus.vel_y = 0;
            }
        }

        bonus.timer -= 1;
        bonus.timer <= 0
    }

    /// Consume the rand calls that C++ `SObjectType::create()` makes when a bonus expires.
    ///
    /// Mirrors the rand sequence in `src/game/sobject.cpp:SObjectType::create()`.
    /// The actual objects are not created (headless sim), but the rand state must advance
    /// identically to C++.
    ///
    /// # Rand accounting (per sobject type)
    /// 1. If `startSound >= 0`: `rand(numSounds)` — 1 call.
    /// 2. If `damage > 0`: for each alive worm within `detectRange` of (x, y):
    ///    - `powerSum = detectRange - |wix - x|`
    ///    - `bloodAmount = blood_pct * powerSum / 100`  (blood_pct = 100 in default settings)
    ///    - For each blood particle: `rand(128)` + `consume_nobject_create2_rand(blood_idx)`
    ///    - `rand(3)` hurt-sound check; if 0: `rand(3)` again.
    ///    For each anyDirt pixel in the `detectRange/2`-wide square around (x,y):
    ///    - `rand(8)` — if 0: `rand(128)` + `consume_nobject_create2_rand(particle_idx)`.
    /// 3. If `dirtEffect >= 0`: `rand(tex.rFrame)` — 1 call.
    fn consume_bonus_expiry_rand(&mut self, bonus_frame: i32, x: i32, y: i32) {
        // Resolve sobject index from bonus frame's sobj name.
        let sobj_name = self.tc.data.constants.bonuses
            .get(bonus_frame as usize)
            .and_then(|b| b.sobj.as_deref());

        let sobj_idx = sobj_name.and_then(|name| {
            self.tc.data.types.sobjects.iter().position(|n| n == name)
        });

        let sobj_idx = match sobj_idx {
            Some(i) => i,
            None    => return, // unknown sobject — no rand calls
        };
        self.consume_sobject_rand_by_idx(sobj_idx, x, y);
    }

    /// Consume the rand calls that `SObjectType::create()` makes, addressed by index.
    ///
    /// Mirrors the rand consumption of `sobjectTypes[idx].create()` in C++.
    fn consume_sobject_rand_by_idx(&mut self, sobj_idx: usize, x: i32, y: i32) {
        if sobj_idx >= self.tc.sobjects.len() { return; }

        let sobj = self.tc.sobjects[sobj_idx].clone();

        // 1. Sound rand + emit explosion event.
        if sobj.start_sound >= 0 {
            self.rand.rand(sobj.num_sounds as u32);
            self.sound_events.push(SoundEvent::Explosion { sobj_idx });
        }

        // 2. Damage effects (worms and dirt particles).
        if sobj.damage > 0 {
            let dr = sobj.detect_range;

            // 2a. Worm damage and blood particles.
            // Resolve blood nobject index (type "blood").
            let blood_nobj_idx = self.tc.data.types.nobjects.iter()
                .position(|n| n == "blood")
                .unwrap_or(6);

            for wi in 0..self.worms.len() {
                let wix = self.worms[wi].pos.x.to_int();
                let wiy = self.worms[wi].pos.y.to_int();
                if wix < x + dr && wix > x - dr && wiy < y + dr && wiy > y - dr {
                    // C++: powerSum = (powerX + powerY) / 2
                    //   powerX = detectRange - |wix - x|
                    //   powerY = detectRange - |wiy - y|
                    let power_x = dr - (wix - x).abs();
                    let power_y = dr - (wiy - y).abs();
                    let power_sum = (power_x + power_y) / 2;
                    // Apply blood_pct scaling.
                    let blood_amount = (self.blood_pct * power_sum / 100) as usize;

                    if self.worms[wi].health > 0 {
                        // Extract worm state before mutable borrow of self.
                        let worm_pos = self.worms[wi].pos;
                        let worm_vel = self.worms[wi].vel;
                        let worm_x = wix;
                        let worm_y = wiy;
                        for _ in 0..blood_amount {
                            let angle = self.rand.rand(128) as usize;
                            // C++: vel_in = w.vel / 3 (raw fixed-point divide by 3)
                            let vel_in = FixedVec { x: Fixed(worm_vel.x.0 / 3), y: Fixed(worm_vel.y.0 / 3) };
                            self.create_nobject2(blood_nobj_idx, angle, vel_in, worm_pos);
                        }
                        // Paint blood stain on terrain near worm.
                        self.paint_blood_stain(worm_x, worm_y);
                    }

                    // Hurt sound.
                    if self.rand.rand(3) == 0 {
                        self.rand.rand(3);
                    }

                    // Apply explosion damage (radial falloff).
                    let damage = sobj.damage;
                    let z = if dr > 0 { damage * power_sum / dr } else { damage };
                    self.do_damage(wi, z, -1);

                    // Knockback (C++: vel.x += blowAway * power if |vel.x| < itof(2)).
                    let blow = sobj.blow_away;
                    if blow != 0 {
                        let delta_x = wix - x;
                        let delta_y = wiy - y;
                        let pw_x = dr - delta_x.abs();
                        let pw_y = dr - delta_y.abs();
                        let v = &mut self.worms[wi].vel;
                        if v.x.0.abs() < (2 << 16) {
                            v.x.0 += if delta_x > 0 { blow * pw_x } else { -blow * pw_x };
                        }
                        if v.y.0.abs() < (2 << 16) {
                            v.y.0 += if delta_y > 0 { blow * pw_y } else { -blow * pw_y };
                        }
                    }
                }
            }

            // 2b. Dirt particle emission from terrain.
            let particle_nobj_idx = self.tc.data.types.nobjects.iter()
                .position(|n| n == "particle__disappearing")
                .unwrap_or(2); // index 2 in openliero TC

            let width = dr / 2;
            let x1 = (x - width).max(0);
            let y1 = (y - width).max(0);
            let x2 = (x + width + 1).min(self.level.width as i32);
            let y2 = (y + width + 1).min(self.level.height as i32);

            let mut _dirt_count = 0usize;
            let mut _dirt_emit_count = 0usize;
            for cy in y1..y2 {
                for cx in x1..x2 {
                    if self.level.material(cx, cy, &self.tc_materials).any_dirt() {
                        _dirt_count += 1;
                        if self.rand.rand(8) == 0 {
                            _dirt_emit_count += 1;
                            let angle = self.rand.rand(128) as usize;
                            // C++: vel_in = fixedvec() = (0,0), pos = itof(ivec2(x, y))
                            let particle_pos = FixedVec {
                                x: Fixed::from_int(cx),
                                y: Fixed::from_int(cy),
                            };
                            self.create_nobject2(particle_nobj_idx, angle, FixedVec::ZERO, particle_pos);
                        }
                    }
                }
            }
        }

        // 3. drawDirtEffect: consume rand AND apply the texture blit to the level.
        //    C++ drawDirtEffect(rand, level, dirtEffect, x-7, y-7) modifies terrain,
        //    which affects the dirt scan of any chained explosion (see section 4 below).
        if sobj.dirt_effect >= 0 {
            let de = sobj.dirt_effect as usize;
            let rframe = self.tc.data.constants.textures
                .get(de)
                .map(|t| t.rframe as u32)
                .unwrap_or(0);
            let rand_result = self.rand.rand(rframe);
            self.apply_dirt_effect(de, x - 7, y - 7, rand_result);
        }

        // 3c. Paint scorch marks around explosion center.
        if sobj.damage > 0 {
            self.paint_scorch_marks(x, y);
        }

        // 3b. Create active SObject for rendering.
        self.sobjects.push(SObject {
            x: x - 8,
            y: y - 8,
            type_idx: sobj_idx,
            cur_frame: 0,
            anim_delay_left: sobj.anim_delay,
        });

        // Apply screen flash (take max, matching C++ `if flash > screenFlash`).
        if sobj.flash > self.screen_flash {
            self.screen_flash = sobj.flash;
        }
        // Apply viewport shake (fixed-point: itof(shake) = shake << 16).
        // Simplified: apply to all viewports (C++ checks if sobject is within viewport bounds).
        if sobj.shake > 0 {
            let shake_fp = sobj.shake << 16;
            for vs in &mut self.viewport_shake {
                if shake_fp > *vs { *vs = shake_fp; }
            }
        }

        // 4. Bonus chain: C++ sobjectTypes::create() iterates all bonuses AFTER the
        //    dirtEffect and triggers sobjectTypes[0] (large_explosion) for each bonus
        //    within detectRange.  The expiring bonus itself is still in the pool at
        //    this point (C++ frees it here via game.bonuses.free(br)), so it will
        //    always match (same position as x,y, and detectRange > 0).
        {
            let chain_dr = sobj.detect_range;
            let mut bi = 0;
            while bi < self.bonuses.len() {
                // Extract position before any mutation (to satisfy borrow checker).
                let pos = self.bonuses[bi].as_ref().map(|b| (b.x >> 16, b.y >> 16));
                bi += 1;
                if let Some((bx, by)) = pos {
                    // C++ check: ix > x - detectRange && ix < x + detectRange (strict)
                    if bx > x - chain_dr && bx < x + chain_dr
                        && by > y - chain_dr && by < y + chain_dr
                    {
                        self.bonuses[bi - 1] = None; // free bonus (mirrors game.bonuses.free(br))
                        self.consume_sobject_rand_by_idx(0, bx, by); // always sobjectTypes[0]
                    }
                }
            }
        }
    }

    /// Port of `NObjectType::create2()` — create a particle NObject.
    ///
    /// Mirrors the rand sequence AND creates the NObject in `self.nobjects`.
    ///
    /// C++ `create2(game, angle, vel_in, pos, color, ownerIdx, firedBy)`:
    ///   1. `realSpeed = speed - rand(speedV)`
    ///   2. `vel = vel_in + cossinTable[angle] * realSpeed / 100`
    ///   3. If `distribution`: `vel.x += rand(dist*2) - dist`, `vel.y += rand(dist*2) - dist`
    ///   4. `create(game, vel, pos, ...)`:
    ///      - If `startFrame > 0`: `rand(numFrames + 1)` (curFrame — visual only)
    ///      - `timeLeft = timeToExplo`
    ///      - If `timeToExploV`: `timeLeft -= rand(timeToExploV)`
    ///   5. `obj.pos += obj.vel`  (initial position advance)
    fn create_nobject2(&mut self, nobj_idx: usize, angle: usize, vel_in: FixedVec, pos: FixedVec) {
        let Some(nobj_type) = self.tc.nobjects.get(nobj_idx) else { return; };
        let nobj_type = nobj_type.clone();

        // 1. Speed rand.
        let real_speed = nobj_type.speed - self.rand.rand(nobj_type.speed_v as u32) as i32;

        // 2. Direction from COSSIN_TABLE.
        let (cos_x, cos_y) = COSSIN_TABLE[angle & 0x7f];
        let dx = Fixed(((cos_x as i64 * real_speed as i64) / 100) as i32);
        let dy = Fixed(((cos_y as i64 * real_speed as i64) / 100) as i32);
        let mut vel = FixedVec { x: vel_in.x + dx, y: vel_in.y + dy };

        // 3. Distribution perturbation.
        if nobj_type.distribution != 0 {
            let dist = nobj_type.distribution as u32;
            vel.x = Fixed(vel.x.0 + self.rand.rand(dist * 2) as i32 - nobj_type.distribution);
            vel.y = Fixed(vel.y.0 + self.rand.rand(dist * 2) as i32 - nobj_type.distribution);
        }

        // 4a. startFrame > 0: pick random animation frame; else use colorBullets as pixel colour.
        let cur_frame = if nobj_type.start_frame > 0 {
            self.rand.rand(nobj_type.num_frames as u32 + 1) as i32
        } else {
            nobj_type.color_bullets
        };

        // 4b. timeToExplo / timeToExploV.
        let mut time_left = nobj_type.time_to_explo;
        if nobj_type.time_to_explo_v != 0 {
            time_left -= self.rand.rand(nobj_type.time_to_explo_v as u32) as i32;
        }

        // 5. Initial position advance: obj.pos = pos + vel (C++: obj.pos = pos; obj.pos += obj.vel).
        let final_pos = pos + vel;

        let mut nobj = NObject::new(nobj_idx, final_pos, vel, time_left);
        nobj.cur_frame = cur_frame;
        self.nobjects.push(nobj);
    }

    /// Port of C++ `drawDirtEffect(common, rand, level, dirtEffect, x, y)`.
    ///
    /// Blits the texture sprite at index `dirtEffect` onto the level starting at
    /// pixel `(x, y)` (note: callers pass `sobject.x - 7, sobject.y - 7`).
    /// The `rand_result` was already consumed by the caller via `rand(tex.rFrame)`.
    ///
    /// Modifies `self.level` pixels so that subsequent dirt scans within the same
    /// frame (e.g. the bonus-chain explosion) see the updated terrain.
    fn apply_dirt_effect(&mut self, dirt_effect: usize, x: i32, y: i32, rand_result: u32) {
        use liero_data::{SPRITE_W, SPRITE_H, SPRITE_SIZE};

        let Some(tex) = self.tc.data.constants.textures.get(dirt_effect) else { return; };
        let ndrawback = tex.ndrawback;
        let sframe    = tex.sframe as usize;
        let mframe    = tex.mframe as usize;
        let rframe    = tex.rframe as usize;

        // Number of total sprite frames available.
        let n_frames = self.tc.large_sprites.len() / SPRITE_SIZE;
        let tframe_idx = sframe + (rand_result as usize % rframe.max(1));
        if tframe_idx >= n_frames || mframe >= n_frames { return; }

        // 16×16 blit region clipped to `(0, 0, width, height-1)`.
        // CLIP_IMAGE in C++ uses `gvl::rect(0, 0, level.width, level.height-1)`.
        let clip_w = self.level.width as i32;
        let clip_h = self.level.height as i32 - 1;

        let mut bx     = x;
        let mut by     = y;
        let mut bw     = SPRITE_W as i32;
        let mut bh     = SPRITE_H as i32;
        let mut m_off  = 0i32; // byte offset into mask frame due to top/left clipping

        // Clip top.
        if by < 0 {
            m_off -= by * SPRITE_W as i32; // skip rows in mask
            bh += by;
            by = 0;
        }
        // Clip bottom.
        let bottom = by + bh - clip_h;
        if bottom > 0 { bh -= bottom; }
        // Clip left.
        if bx < 0 {
            m_off -= bx; // skip columns in mask row
            bw += bx;
            bx = 0;
        }
        // Clip right.
        let right = bx + bw - clip_w;
        if right > 0 { bw -= right; }

        if bw <= 0 || bh <= 0 { return; }

        let center_x = Fixed::from_int(x + 7);  // Center of 16×16 blit
        let center_y = Fixed::from_int(y + 7);
        let mut debris_count = 0;

        {
            let mframe_data = &self.tc.large_sprites[mframe * SPRITE_SIZE .. (mframe + 1) * SPRITE_SIZE];
            let tframe_data = &self.tc.large_sprites[tframe_idx * SPRITE_SIZE .. (tframe_idx + 1) * SPRITE_SIZE];

            for y_ in 0..bh {
                for x_ in 0..bw {
                    let mask_idx = (m_off + y_ * SPRITE_W as i32 + x_) as usize;
                    let c = mframe_data[mask_idx];

                    let lx = bx + x_;
                    let ly = by + y_;

                    if ndrawback {
                        match c {
                            6 => {
                                // Replace dirt pixels with texture sprite pixels.
                                if self.level.material(lx, ly, &self.tc_materials).any_dirt() {
                                    let mx = lx as usize;
                                    let my = ly as usize;
                                    let new_pix = tframe_data[((my & 15) << 4) + (mx & 15)];
                                    self.level.set_pixel(lx, ly, new_pix);
                                }
                            }
                            1 => {
                                // Normalize dirt colour (keep material, standardise palette index).
                                let mat = self.level.material(lx, ly, &self.tc_materials);
                                if mat.0 & 2 != 0 {
                                    // Dirt2
                                    self.level.set_pixel(lx, ly, 2);
                                } else if mat.dirt() {
                                    self.level.set_pixel(lx, ly, 1);
                                }
                            }
                            _ => {}
                        }
                    } else {
                        match c {
                            10 | 6 => {
                                // Replace background pixels with texture sprite pixels.
                                if self.level.material(lx, ly, &self.tc_materials).background() {
                                    let mx = lx as usize;
                                    let my = ly as usize;
                                    let new_pix = tframe_data[((my & 15) << 4) + (mx & 15)];
                                    self.level.set_pixel(lx, ly, new_pix);
                                    // Count pixels cleared for particle spawning
                                    debris_count += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Spawn particles after sprite data references are dropped
        for _ in 0..debris_count / 5 {
            self.spawn_debris_particle(center_x, center_y);
        }

        // Spawn at least 3-5 particles even if not many pixels cleared
        let particle_count: i32 = (debris_count / 5).max(3).min(5);
        for _ in 0..particle_count.saturating_sub(debris_count / 5) {
            self.spawn_debris_particle(center_x, center_y);
        }
    }

    /// Spawn a small "dust" debris particle from explosion terrain clearing.
    fn spawn_debris_particle(&mut self, center_x: Fixed, center_y: Fixed) {
        let dust_idx = self.tc.data.types.nobjects.iter()
            .position(|n| n == "dust")
            .unwrap_or(2); // fallback to index 2 if "dust" not found

        let angle = self.rand.rand(128) as usize;
        let (cos_x, cos_y) = COSSIN_TABLE[angle & 0x7f];

        // Outward velocity ~1-2 pixels/frame in fixed-point
        let speed_range = 65536 / 2; // ~0.5 pixels/frame in fixed-point
        let speed = 65536 + self.rand.rand(speed_range as u32) as i32; // 1.0-1.5 px/frame
        let vel_x = Fixed(((cos_x as i64 * speed as i64) / 100) as i32);
        let vel_y = Fixed(((cos_y as i64 * speed as i64) / 100) as i32);

        let vel_in = FixedVec { x: vel_x, y: vel_y };
        self.create_nobject2(dust_idx, angle, vel_in, FixedVec { x: center_x, y: center_y });
    }

    /// Deposit dirt into background pixels (inverse of `apply_dirt_effect`).
    /// Used by weapons with `dirt_deposit = true` (e.g. Dirt Cannon).
    fn apply_dirt_deposit(&mut self, dirt_effect: usize, x: i32, y: i32, rand_result: u32) {
        use liero_data::{SPRITE_W, SPRITE_H, SPRITE_SIZE};

        let Some(tex) = self.tc.data.constants.textures.get(dirt_effect) else { return; };
        let sframe = tex.sframe as usize;
        let rframe = tex.rframe as usize;

        let n_frames = self.tc.large_sprites.len() / SPRITE_SIZE;
        let tframe_idx = sframe + (rand_result as usize % rframe.max(1));
        if tframe_idx >= n_frames { return; }

        let clip_w = self.level.width as i32;
        let clip_h = self.level.height as i32 - 1;

        let mut bx = x;
        let mut by = y;
        let mut bw = SPRITE_W as i32;
        let mut bh = SPRITE_H as i32;

        if by < 0 { bh += by; by = 0; }
        let bottom = by + bh - clip_h;
        if bottom > 0 { bh -= bottom; }
        if bx < 0 { bw += bx; bx = 0; }
        let right = bx + bw - clip_w;
        if right > 0 { bw -= right; }
        if bw <= 0 || bh <= 0 { return; }

        let tframe_data = &self.tc.large_sprites[tframe_idx * SPRITE_SIZE .. (tframe_idx + 1) * SPRITE_SIZE];

        for y_ in 0..bh {
            for x_ in 0..bw {
                let lx = bx + x_;
                let ly = by + y_;
                // Only paint background (empty) pixels — don't overwrite existing terrain.
                if self.level.material(lx, ly, &self.tc_materials).background() {
                    let mx = lx as usize;
                    let my = ly as usize;
                    let new_pix = tframe_data[((my & 15) << 4) + (mx & 15)];
                    self.level.set_pixel(lx, ly, new_pix);
                }
            }
        }
    }

    /// Port of `NObject::process()` for all live nobjects — `src/game/nobject.cpp:76`.
    ///
    /// Called BEFORE `++cycles` in the frame, same as C++ `processFrame` order:
    /// bonuses → sobjects → wobjects → **nobjects** → ++cycles → bonuses spawn → worms.
    ///
    /// Headless simplification: skips bobject blood-trail creation, sound effects,
    /// and per-worm hitDamage (all are unpredictable / visual).
    ///
    /// For blood and particle__disappearing (the only NObjects currently created),
    /// bounce=0 and hitDamage=0 so those branches are dead code anyway.
    fn step_nobjects(&mut self) {
        let mut i = 0;
        while i < self.nobjects.len() {
            // Clone type data to avoid borrowing self.tc and self.nobjects simultaneously.
            let ntype = match self.tc.nobjects.get(self.nobjects[i].nobj_type) {
                Some(t) => t.clone(),
                None    => { i += 1; continue; }
            };

            // ── 1. pos += vel ────────────────────────────────────────────────
            let vel = self.nobjects[i].vel;
            self.nobjects[i].pos += vel;

            // ── 1.5. Blood trail (spawn every blood_trail_delay frames) ───────
            if ntype.blood_trail && ntype.blood_trail_delay > 0 {
                if (self.cycles % ntype.blood_trail_delay.max(1)) == 0 {
                    // Find blood nobject type by name, or use hardcoded fallback
                    let blood_idx = self.tc.data.types.nobjects.iter()
                        .position(|name| name == "blood")
                        .unwrap_or(6); // Fallback to index 6 if "blood" not found

                    if blood_idx < self.tc.nobjects.len() {
                        let trail_pos = self.nobjects[i].pos;
                        // Spawn blood trail at nobject position with reduced velocity (vel / 4)
                        let trail_vel = FixedVec {
                            x: Fixed(self.nobjects[i].vel.x.0 / 4),
                            y: Fixed(self.nobjects[i].vel.y.0 / 4),
                        };
                        self.create_nobject2(blood_idx, 0, trail_vel, trail_pos);
                    }
                }
            }

            // ── 2. Bounce ────────────────────────────────────────────────────
            if ntype.bounce > 0 {
                let pos = self.nobjects[i].pos;
                let vel = self.nobjects[i].vel;
                let inew_x = (pos + vel).x.to_int();
                let inew_y = (pos + vel).y.to_int();
                let ipos_x = pos.x.to_int();
                let ipos_y = pos.y.to_int();

                // X collision: check (inew_x, ipos_y).
                if !self.level.inside(inew_x, ipos_y)
                    || self.level.material(inew_x, ipos_y, &self.tc_materials).dirt_rock()
                {
                    let v = &mut self.nobjects[i].vel;
                    v.x = Fixed((-v.x.0).wrapping_mul(ntype.bounce) / 100);
                    v.y = Fixed((v.y.0 * 4) / 5); // TODO: read from TC
                }

                // Y collision: check (ipos_x, inew_y).
                if !self.level.inside(ipos_x, inew_y)
                    || self.level.material(ipos_x, inew_y, &self.tc_materials).dirt_rock()
                {
                    let v = &mut self.nobjects[i].vel;
                    v.y = Fixed((-v.y.0).wrapping_mul(ntype.bounce) / 100);
                    v.x = Fixed((v.x.0 * 4) / 5); // TODO: read from TC
                }
            }

            // ── 3. Recompute inewPos (after possible bounce adjustment) ──────
            let pos = self.nobjects[i].pos;
            let vel = self.nobjects[i].vel;
            let inew = pos + vel;
            let inew_x = inew.x.to_int();
            let inew_y = inew.y.to_int();

            // ── 4. Clamp pos if inewPos is OOB ───────────────────────────────
            {
                let p = &mut self.nobjects[i].pos;
                if inew_x < 0                          { p.x = Fixed(0); }
                if inew_y < 0                          { p.y = Fixed(0); }
                if inew_x >= self.level.width  as i32  { p.x = Fixed::from_int(self.level.width  as i32); }
                if inew_y >= self.level.height as i32  { p.y = Fixed::from_int(self.level.height as i32); }
            }

            // ── 5. Terrain collision (or OOB) ─────────────────────────────────
            let in_terrain = !self.level.inside(inew_x, inew_y)
                || self.level.material(inew_x, inew_y, &self.tc_materials).dirt_rock();

            let mut do_explode = if in_terrain {
                self.nobjects[i].vel = FixedVec::ZERO;
                ntype.expl_ground
            } else {
                self.nobjects[i].vel.y += Fixed(ntype.gravity);
                false
            };

            // ── 6. timeToExplo countdown ─────────────────────────────────────
            if ntype.time_to_explo > 0 {
                self.nobjects[i].time_left -= 1;
                if self.nobjects[i].time_left <= 0 {
                    do_explode = true;
                }
            }

            // ── 7. hitDamage: check worm collisions ──────────────────────────
            if ntype.hit_damage > 0 {
                let nobject_pos = self.nobjects[i].pos;
                let nobject_vel = self.nobjects[i].vel;
                let detect_dist_sq = (ntype.detect_distance as i64) * (ntype.detect_distance as i64);

                let mut hit_any = false;
                for wi in 0..self.worms.len() {
                    if !self.worms[wi].alive { continue; }

                    let worm_pos = self.worms[wi].pos;
                    let dx = (nobject_pos.x.to_int() - worm_pos.x.to_int()) as i64;
                    let dy = (nobject_pos.y.to_int() - worm_pos.y.to_int()) as i64;
                    let dist_sq = dx * dx + dy * dy;

                    if dist_sq <= detect_dist_sq {
                        // Hit! Apply damage
                        self.do_damage(wi, ntype.hit_damage, -1);

                        // Apply knockback
                        self.worms[wi].vel.x.0 = (self.worms[wi].vel.x.0 as i64
                            + (nobject_vel.x.0 as i64 * ntype.blow_away as i64 / 100)) as i32;
                        self.worms[wi].vel.y.0 = (self.worms[wi].vel.y.0 as i64
                            + (nobject_vel.y.0 as i64 * ntype.blow_away as i64 / 100)) as i32;

                        // Spawn blood nobjects
                        if ntype.blood_on_hit > 0 {
                            let blood_idx = self.tc.data.types.nobjects.iter()
                                .position(|n| n == "blood").unwrap_or(6);
                            for _ in 0..ntype.blood_on_hit {
                                let angle = self.rand.rand(128) as usize;
                                let vel_in = FixedVec {
                                    x: Fixed(nobject_vel.x.0 / 3),
                                    y: Fixed(nobject_vel.y.0 / 3)
                                };
                                self.create_nobject2(blood_idx, angle, vel_in, nobject_pos);
                            }
                        }

                        hit_any = true;
                        break; // Only hit once per nobject per frame
                    }
                }

                // Handle explosion/destruction after hitting a worm
                if hit_any {
                    if ntype.worm_explode {
                        do_explode = true;
                    } else if ntype.worm_destroy {
                        self.nobjects.remove(i);
                        continue; // Skip remaining checks for this nobject
                    }
                }
            }

            // ── 8. Explode: createOnExp, dirtEffect, splinterAmount ───────────
            if do_explode {
                if let Some(ref exp_name) = ntype.create_on_exp.clone() {
                    let sobj_idx = self.tc.data.types.sobjects.iter().position(|n| n == exp_name);
                    if let Some(idx) = sobj_idx {
                        let exp_x = self.nobjects[i].pos.x.to_int();
                        let exp_y = self.nobjects[i].pos.y.to_int();
                        self.consume_sobject_rand_by_idx(idx, exp_x, exp_y);
                    }
                }
                if ntype.dirt_effect >= 0 {
                    let de = ntype.dirt_effect as usize;
                    let rframe = self.tc.data.constants.textures.get(de).map(|t| t.rframe as u32).unwrap_or(0);
                    let rand_result = self.rand.rand(rframe);
                    let exp_x = self.nobjects[i].pos.x.to_int();
                    let exp_y = self.nobjects[i].pos.y.to_int();
                    self.apply_dirt_effect(de, exp_x - 7, exp_y - 7, rand_result);
                }
                if ntype.splinter_amount > 0 {
                    if let Some(ref stype_name) = ntype.splinter_type.clone() {
                        let snobj_idx = self.tc.data.types.nobjects.iter().position(|n| n == stype_name);
                        if let Some(sidx) = snobj_idx {
                            let spos = self.nobjects[i].pos;
                            let scol = ntype.splinter_colour;
                            for _ in 0..ntype.splinter_amount {
                                let angle = self.rand.rand(128) as usize;
                                let color_sub = self.rand.rand(2) as i32;
                                let _color = scol - color_sub; // visual only
                                self.create_nobject2(sidx, angle, FixedVec::ZERO, spos);
                            }
                        }
                    }
                }
                // Remove this NObject.
                self.nobjects.remove(i);
                // Don't increment i — the shifted element needs processing.
            } else {
                i += 1;
            }
        }
    }

    /// Update floating damage indicators — drift upward, decelerate, and expire.
    fn step_floating_damages(&mut self) {
        self.floating_damages.retain_mut(|fd| {
            fd.y += fd.vel_y;
            fd.vel_y = fd.vel_y * 95 / 100; // decelerate by 5%
            fd.timer -= 1;
            fd.timer > 0
        });
    }

    /// Apply `amount` damage to worm `worm_idx`, triggering death if health drops to zero.
    ///
    /// `by_idx` = attacker worm index, or -1 for environmental damage.
    ///
    /// Respects game-mode rules:
    /// - **TDM**: skips friendly-fire (same team_id).
    /// - **Juggernaut**: attacker juggernaut deals ×2; victim juggernaut receives ×½.
    /// - On death: delegates to `on_worm_death()` for mode-specific effects.
    fn do_damage(&mut self, worm_idx: usize, amount: i32, by_idx: i32) {
        if amount <= 0 { return; }

        // Skip eliminated or dead worms.
        if !self.worms[worm_idx].alive || self.worms[worm_idx].eliminated { return; }

        // Friendly fire check: skip damage if teams are the same and friendly_fire is disabled.
        if !self.friendly_fire && by_idx >= 0 {
            let a_team = self.worms.get(by_idx as usize).map(|w| w.team_id).unwrap_or(-1);
            let v_team = self.worms[worm_idx].team_id;
            if a_team >= 0 && a_team == v_team { return; }
        }

        // TDM always blocks same-team damage in the old hardcoded way (fallback).
        if matches!(self.mode, GameMode::TeamDeathmatch) && self.friendly_fire {
            if by_idx >= 0 {
                let a_team = self.worms.get(by_idx as usize).map(|w| w.team_id).unwrap_or(-1);
                let v_team = self.worms[worm_idx].team_id;
                if a_team >= 0 && a_team == v_team { return; }
            }
        }

        // Juggernaut damage multipliers.
        let attacker_is_jug = by_idx >= 0
            && self.worms.get(by_idx as usize).map(|w| w.is_juggernaut).unwrap_or(false);
        let victim_is_jug = self.worms[worm_idx].is_juggernaut;
        let mut actual = amount;
        if attacker_is_jug { actual *= 2; }
        if victim_is_jug   { actual /= 2; }
        // Kevlar: 50% damage reduction while active.
        if self.worms[worm_idx].kevlar_timer > 0 { actual /= 2; }
        actual = actual.max(1);

        self.worms[worm_idx].health -= actual;
        self.worms[worm_idx].damage_flash = 8;

        // ScalesOfJustice: attacker heals by damage amount
        if matches!(self.mode, GameMode::ScalesOfJustice) && by_idx >= 0 {
            let attacker_idx = by_idx as usize;
            if attacker_idx < self.worms.len() {
                let max_health = self.worm_health;
                let old_health = self.worms[attacker_idx].health;
                let new_health = old_health + actual;

                if new_health > max_health {
                    // Gain a life for overflow
                    self.worms[attacker_idx].health = new_health - max_health;
                    self.worms[attacker_idx].lives += 1;
                } else {
                    self.worms[attacker_idx].health = new_health;
                }
            }
        }

        // Spawn floating damage indicator.
        self.floating_damages.push(FloatingDamage {
            x: self.worms[worm_idx].pos.x.0,
            y: self.worms[worm_idx].pos.y.0 - Fixed::from_int(10).0,
            amount: actual,
            timer: 40,
            vel_y: -30000, // upward velocity in fixed-point Q16.16
        });

        if self.worms[worm_idx].health <= 0 {
            self.worms[worm_idx].health       = 0;
            self.worms[worm_idx].alive        = false;
            self.worms[worm_idx].killed_timer = 150;
            self.worms[worm_idx].lives        -= 1;
            self.worms[worm_idx].deaths       += 1;

            // Spawn blood particles when worm dies (scaled by blood_pct).
            {
                let blood_idx = self.tc.data.types.nobjects.iter()
                    .position(|n| n == "blood").unwrap_or(6);
                let dead_pos = self.worms[worm_idx].pos;
                let dead_vel = self.worms[worm_idx].vel;
                let dead_x = dead_pos.x.to_int();
                let dead_y = dead_pos.y.to_int();
                let blood_count = (120 * self.blood_pct / 100).max(1) as usize;
                for _ in 0..blood_count {
                    let angle = self.rand.rand(128) as usize;
                    let vel_in = FixedVec {
                        x: Fixed(dead_vel.x.0 / 3),
                        y: Fixed(dead_vel.y.0 / 3),
                    };
                    self.create_nobject2(blood_idx, angle, vel_in, dead_pos);
                }
                // Paint blood stain on terrain.
                self.paint_blood_stain(dead_x, dead_y);
            }

            // Kill credit.
            if by_idx >= 0 {
                let a = by_idx as usize;
                if a != worm_idx && a < self.worms.len() {
                    self.worms[a].kills += 1;
                    // GameOfTag: set tag_worm to the killer and reset their timer
                    if matches!(self.mode, GameMode::GameOfTag { .. }) {
                        self.tag_worm = Some(a);
                        self.worms[a].timer = 0;
                    }
                }
            }

            self.sound_events.push(SoundEvent::Death { worm_idx });
            self.on_worm_death(worm_idx, by_idx);
        } else {
            self.sound_events.push(SoundEvent::Hurt { worm_idx });
        }
    }

    /// Mode-specific hooks triggered on worm death (called from `do_damage`).
    fn on_worm_death(&mut self, worm_idx: usize, by_idx: i32) {
        match self.mode.clone() {
            GameMode::ZombieMode => {
                // Non-zombies convert; zombies just respawn normally.
                if !self.worms[worm_idx].is_zombie {
                    self.worms[worm_idx].is_zombie = true;
                    // Restore lives so zombie can always respawn.
                    self.worms[worm_idx].lives = 999;
                }
            }
            GameMode::BombTag { .. } => {
                if self.worms[worm_idx].has_bomb {
                    self.worms[worm_idx].has_bomb = false;
                    // Transfer bomb to nearest alive worm.
                    let dx = self.worms[worm_idx].pos.x.to_int();
                    let dy = self.worms[worm_idx].pos.y.to_int();
                    let new_holder = (0..self.worms.len())
                        .filter(|&i| i != worm_idx && self.worms[i].alive)
                        .min_by_key(|&i| {
                            let ex = self.worms[i].pos.x.to_int();
                            let ey = self.worms[i].pos.y.to_int();
                            (ex - dx).pow(2) + (ey - dy).pow(2)
                        });
                    if let Some(h) = new_holder {
                        self.worms[h].has_bomb = true;
                    }
                }
            }
            GameMode::Juggernaut => {
                if self.worms[worm_idx].is_juggernaut {
                    self.worms[worm_idx].is_juggernaut = false;
                    // Crown passes to killer, or to next alive worm.
                    let new_jug = if by_idx >= 0 && (by_idx as usize) < self.worms.len()
                        && self.worms[by_idx as usize].alive
                    {
                        by_idx as usize
                    } else {
                        (0..self.worms.len())
                            .find(|&i| i != worm_idx && self.worms[i].alive)
                            .unwrap_or(0)
                    };
                    self.worms[new_jug].is_juggernaut = true;
                    self.worms[new_jug].health = (self.worm_health * 3).min(999);
                }
            }
            GameMode::GunGame => {
                // Advance the killer's weapon slot on kill
                if by_idx >= 0 && (by_idx as usize) < self.worms.len() {
                    let killer_idx = by_idx as usize;
                    let next_slot = (self.worms[killer_idx].current_weapon + 1) % 5;
                    self.worms[killer_idx].current_weapon = next_slot;
                }
            }
            _ => {}
        }
    }

    /// Advance all active wobjects by one frame.
    ///
    /// Implements STNormal physics (and basic shared logic for all shot types).
    /// Advanced shot types (Homing, Steerable, Laser, DType2) are treated as STNormal
    /// for now (R2 scope for steering).
    ///
    /// Mirrors `WObject::process()` in `src/game/weapon.cpp`.
    fn step_wobjects(&mut self, inputs: &[u32]) {
        let mut i = 0;
        while i < self.wobjects.len() {
            let weapon = match self.tc.weapons.get(self.wobjects[i].weapon_idx) {
                Some(w) => w.clone(),
                None    => { i += 1; continue; }
            };

            // ── 1. pos += vel ─────────────────────────────────────────────────
            let vel = self.wobjects[i].vel;
            self.wobjects[i].pos += vel;

            // ── 1.0. Remote detonation: CHANGE + FIRE triggers immediate detonation
            if weapon.rem_exp_object {
                let owner_idx = self.wobjects[i].owner_idx;
                if owner_idx < self.worms.len() {
                    let inp = inputs.get(owner_idx).copied().unwrap_or(0);
                    if inp & crate::worm::input::CHANGE != 0 && inp & crate::worm::input::FIRE != 0 {
                        self.wobjects[i].time_left = 0;
                    }
                }
            }

            // ── 1.1. Shot-type: steerable (STSteerable = 2) ───────────────────
            // Owner holds UP to boost; missile steers toward aim angle each frame.
            if weapon.shot_type == 2 {
                let ang  = (self.wobjects[i].cur_frame & 0x7f) as usize;
                let (cx, cy) = COSSIN_TABLE[ang];
                let base_x   = ((cx as i64 * weapon.speed as i64) / 100) as i32;
                let base_y   = ((cy as i64 * weapon.speed as i64) / 100) as i32;
                let owner    = self.wobjects[i].owner_idx;
                let (boost_x, boost_y) = if owner < self.worms.len() && self.worms[owner].alive {
                    let inp = inputs.get(owner).copied().unwrap_or(0);
                    if inp & crate::worm::input::UP != 0 {
                        let bx = ((cx as i64 * weapon.add_speed as i64) / 100) as i32;
                        let by = ((cy as i64 * weapon.add_speed as i64) / 100) as i32;
                        (bx, by)
                    } else { (0, 0) }
                } else { (0, 0) };
                let new_vx = base_x + boost_x;
                let new_vy = base_y + boost_y;
                let old_vx = self.wobjects[i].vel.x.0;
                let old_vy = self.wobjects[i].vel.y.0;
                // Blend: 8/9 old + 1/9 new to smooth steering.
                self.wobjects[i].vel.x.0 = ((old_vx as i64 * 8 + new_vx as i64) / 9) as i32;
                self.wobjects[i].vel.y.0 = ((old_vy as i64 * 8 + new_vy as i64) / 9) as i32;
            }

            // ── 1.2. Shot-type: homing (STHoming = 5) ─────────────────────────
            // Tracks nearest visible enemy worm using cross-product sign for steer direction.
            if weapon.shot_type == 5 && weapon.homing_strength > 0 {
                let owner = self.wobjects[i].owner_idx;
                let proj_x = self.wobjects[i].pos.x.to_int();
                let proj_y = self.wobjects[i].pos.y.to_int();
                let vel_x  = self.wobjects[i].vel.x.0;
                let vel_y  = self.wobjects[i].vel.y.0;

                // Find nearest non-owner alive worm.
                let mut best_dist = i32::MAX;
                let mut best_wi   = usize::MAX;
                for wi in 0..self.worms.len() {
                    if wi == owner { continue; }
                    if !self.worms[wi].alive { continue; }
                    let wx = self.worms[wi].pos.x.to_int();
                    let wy = self.worms[wi].pos.y.to_int();
                    let d  = crate::math::vector_length(wx - proj_x, wy - proj_y);
                    if d < best_dist { best_dist = d; best_wi = wi; }
                }

                if best_wi < self.worms.len() {
                    let tx = self.worms[best_wi].pos.x.to_int() - proj_x;
                    let ty = self.worms[best_wi].pos.y.to_int() - proj_y;
                    // Cross-product: vel × target.  Positive → target is to the right.
                    let cross = (vel_x as i64) * (ty as i64) - (vel_y as i64) * (tx as i64);
                    let step  = weapon.homing_strength;
                    let ang   = self.wobjects[i].cur_frame;
                    self.wobjects[i].cur_frame = if cross > 0 {
                        (ang + step).rem_euclid(128)
                    } else if cross < 0 {
                        (ang - step).rem_euclid(128)
                    } else { ang };
                    // Re-apply velocity from updated angle.
                    let new_ang = (self.wobjects[i].cur_frame & 0x7f) as usize;
                    let (cx, cy) = COSSIN_TABLE[new_ang];
                    self.wobjects[i].vel.x.0 = ((cx as i64 * weapon.speed as i64) / 100) as i32;
                    self.wobjects[i].vel.y.0 = ((cy as i64 * weapon.speed as i64) / 100) as i32;
                }
            }

            // ── 1.3. mult_speed (velocity decay/growth each frame) ────────────
            if weapon.mult_speed != 100 && weapon.mult_speed != 0 {
                self.wobjects[i].vel.x.0 = ((self.wobjects[i].vel.x.0 as i64 * weapon.mult_speed as i64) / 100) as i32;
                self.wobjects[i].vel.y.0 = ((self.wobjects[i].vel.y.0 as i64 * weapon.mult_speed as i64) / 100) as i32;
            }

            // ── 1.4. Sprite animation update (every 8 cycles) ─────────────────
            if weapon.start_frame >= 0 && weapon.shot_type == 0 && weapon.num_frames > 0 {
                if (self.cycles & 7) == 0 {
                    if weapon.loop_anim {
                        self.wobjects[i].cur_frame = (self.wobjects[i].cur_frame + 1) % (weapon.num_frames + 1);
                    } else if self.wobjects[i].cur_frame < weapon.num_frames {
                        self.wobjects[i].cur_frame += 1;
                    }
                }
            }

            // ── 1.5. Object trail (obj_trail_delay > 0 → spawn sobject) ───────
            if weapon.obj_trail_delay > 0 {
                if (self.cycles % weapon.obj_trail_delay.max(1)) == 0 {
                    if let Some(ref trail_name) = weapon.obj_trail_type.clone() {
                        let tx = self.wobjects[i].pos.x.to_int();
                        let ty = self.wobjects[i].pos.y.to_int();
                        let sobj_idx = self.tc.data.types.sobjects.iter().position(|n| n == trail_name);
                        if let Some(idx) = sobj_idx {
                            self.consume_sobject_rand_by_idx(idx, tx, ty);
                        }
                    }
                }
            }

            // ── 1.6. Particle trail (part_trail_delay > 0 → spawn nobject) ────
            if weapon.part_trail_delay > 0 {
                if (self.cycles % weapon.part_trail_delay.max(1)) == 0 {
                    if let Some(ref trail_name) = weapon.part_trail_obj.clone() {
                        let nobj_idx = self.tc.data.types.nobjects.iter().position(|n| n == trail_name);
                        if let Some(nidx) = nobj_idx {
                            let tpos = self.wobjects[i].pos;
                            let proj_vel = self.wobjects[i].vel;

                            // Part trail velocity based on part_trail_type:
                            // Type 1 ("Larpa"): velocity = projectile velocity / ~3
                            // Other types: velocity = projectile velocity / ~5, or angle-based
                            let trail_vel = if weapon.part_trail_type == 1 {
                                // Larpa: divide by 3 (SplinterLarpaVelDiv in C++)
                                FixedVec {
                                    x: Fixed(proj_vel.x.0 / 3),
                                    y: Fixed(proj_vel.y.0 / 3),
                                }
                            } else if weapon.part_trail_type > 0 {
                                // Other types: divide by 5 (SplinterCracklerVelDiv in C++)
                                FixedVec {
                                    x: Fixed(proj_vel.x.0 / 5),
                                    y: Fixed(proj_vel.y.0 / 5),
                                }
                            } else {
                                // Type 0: angle-based (default behavior)
                                FixedVec::ZERO
                            };

                            if weapon.part_trail_type > 0 {
                                // Velocity-based: use angle 0 with computed velocity
                                self.create_nobject2(nidx, 0, trail_vel, tpos);
                            } else {
                                // Angle-based: random angle with no initial velocity
                                let angle = self.rand.rand(128) as usize;
                                self.create_nobject2(nidx, angle, trail_vel, tpos);
                            }
                        }
                    }
                }
            }

            let mut pos    = self.wobjects[i].pos;
            let mut ipos_x = pos.x.to_int();
            let mut ipos_y = pos.y.to_int();

            // ── 2. Out-of-bounds check: if velocity would move projectile outside level, explode immediately ──
            let next_x = ipos_x + (vel.x.to_int());
            let next_y = ipos_y + (vel.y.to_int());
            let will_be_oob = !self.level.inside(next_x, next_y);

            let mut do_explode = false;
            let mut do_remove  = false;

            if will_be_oob {
                do_explode = true;
            }

            // ── 2. Terrain collision ───────────────────────────────────────────
            let in_terrain = !self.level.inside(ipos_x, ipos_y)
                || self.level.material(ipos_x, ipos_y, &self.tc_materials).dirt_rock();

            if !do_explode && in_terrain && !weapon.pierce_dirt {
                if weapon.bounce > 0 {
                    // Bounce: reflect velocity on the axis that caused the collision.
                    // px_prev/py_prev = position before this frame's movement.
                    let px_prev = Fixed(self.wobjects[i].pos.x.0 - vel.x.0).to_int();
                    let py_prev = Fixed(self.wobjects[i].pos.y.0 - vel.y.0).to_int();
                    // Determine which axis caused entry into terrain:
                    // hit_y = Y-movement caused collision (old_x + new_y is solid)
                    // hit_x = X-movement caused collision (new_x + old_y is solid)
                    let hit_y = self.level.inside(px_prev, ipos_y)
                        && self.level.material(px_prev, ipos_y, &self.tc_materials).dirt_rock();
                    let hit_x = self.level.inside(ipos_x, py_prev)
                        && self.level.material(ipos_x, py_prev, &self.tc_materials).dirt_rock();
                    // Back out of terrain, then update ipos for correct worm-hit test.
                    self.wobjects[i].pos.x.0 -= vel.x.0;
                    self.wobjects[i].pos.y.0 -= vel.y.0;
                    pos    = self.wobjects[i].pos;
                    ipos_x = pos.x.to_int();
                    ipos_y = pos.y.to_int();
                    // Reflect: bounce retains `bounce`% of speed on the hit axis.
                    if hit_x || (!hit_x && !hit_y) {
                        self.wobjects[i].vel.x.0 =
                            -(self.wobjects[i].vel.x.0 as i64 * weapon.bounce as i64 / 100) as i32;
                    }
                    if hit_y || (!hit_x && !hit_y) {
                        self.wobjects[i].vel.y.0 =
                            -(self.wobjects[i].vel.y.0 as i64 * weapon.bounce as i64 / 100) as i32;
                    }
                } else if weapon.expl_ground {
                    do_explode = true;
                } else {
                    // Stop in terrain.
                    self.wobjects[i].vel = FixedVec::ZERO;
                }
            }

            // ── 3. Gravity (only when not embedded in terrain) ────────────────
            if !in_terrain {
                self.wobjects[i].vel.y.0 += weapon.gravity;
            }

            // ── 3.5. Attraction/repulsion force ────────────────────────────────
            if (!in_terrain || weapon.attract_always) && weapon.attract_radius > 0 && weapon.attract_force != 0 {
                // Copy values to avoid borrow conflicts.
                let pos_i = self.wobjects[i].pos;
                let attract_radius = weapon.attract_radius;
                let attract_force = weapon.attract_force;

                // Attract wobjects.
                for j in 0..self.wobjects.len() {
                    if i == j { continue; }
                    let pos_j = self.wobjects[j].pos;
                    let dx = pos_j.x.to_int() - pos_i.x.to_int();
                    let dy = pos_j.y.to_int() - pos_i.y.to_int();
                    let dist = vector_length(dx, dy);

                    if dist > 0 && dist < attract_radius {
                        // force = attract_force * delta / (1000 * dist)
                        let force_x = (attract_force as i64).wrapping_mul(dx as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        let force_y = (attract_force as i64).wrapping_mul(dy as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        self.wobjects[j].vel.x.0 = self.wobjects[j].vel.x.0.wrapping_add(force_x as i32);
                        self.wobjects[j].vel.y.0 = self.wobjects[j].vel.y.0.wrapping_add(force_y as i32);
                    }
                }

                // Attract nobjects.
                for j in 0..self.nobjects.len() {
                    let pos_j = self.nobjects[j].pos;
                    let dx = pos_j.x.to_int() - pos_i.x.to_int();
                    let dy = pos_j.y.to_int() - pos_i.y.to_int();
                    let dist = vector_length(dx, dy);

                    if dist > 0 && dist < attract_radius {
                        let force_x = (attract_force as i64).wrapping_mul(dx as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        let force_y = (attract_force as i64).wrapping_mul(dy as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        self.nobjects[j].vel.x.0 = self.nobjects[j].vel.x.0.wrapping_add(force_x as i32);
                        self.nobjects[j].vel.y.0 = self.nobjects[j].vel.y.0.wrapping_add(force_y as i32);
                    }
                }

                // Attract living worms.
                for j in 0..self.worms.len() {
                    if !self.worms[j].alive { continue; }
                    let pos_j = self.worms[j].pos;
                    let dx = pos_j.x.to_int() - pos_i.x.to_int();
                    let dy = pos_j.y.to_int() - pos_i.y.to_int();
                    let dist = vector_length(dx, dy);

                    if dist > 0 && dist < attract_radius {
                        let force_x = (attract_force as i64).wrapping_mul(dx as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        let force_y = (attract_force as i64).wrapping_mul(dy as i64)
                            / (1000i64 * dist as i64) as i32 as i64;
                        self.worms[j].vel.x.0 = self.worms[j].vel.x.0.wrapping_add(force_x as i32);
                        self.worms[j].vel.y.0 = self.worms[j].vel.y.0.wrapping_add(force_y as i32);
                    }
                }
            }

            // ── 3.6. Boomerang return force ───────────────────────────────────
            if !do_explode && !do_remove && weapon.shot_type == weapon::shot_type::BOOMERANG && weapon.boomerang_return_force > 0 {
                let owner = self.wobjects[i].owner_idx;
                if owner < self.worms.len() && self.worms[owner].alive {
                    let proj  = self.wobjects[i].pos;
                    let opos  = self.worms[owner].pos;
                    let dx    = opos.x.to_int() - proj.x.to_int();
                    let dy    = opos.y.to_int() - proj.y.to_int();
                    let dist  = crate::math::vector_length(dx, dy).max(1);
                    let force = weapon.boomerang_return_force;
                    self.wobjects[i].vel.x.0 += ((force as i64 * dx as i64) / dist as i64) as i32;
                    self.wobjects[i].vel.y.0 += ((force as i64 * dy as i64) / dist as i64) as i32;
                    // Catch: remove when the boomerang reaches its owner.
                    if dist < 10 {
                        do_remove = true;
                    }
                }
            }

            // ── 3.7. Tesla Coil area damage tick (when embedded in terrain) ───
            if !do_explode && !do_remove && weapon.damage_area_tick > 0 && in_terrain {
                self.wobjects[i].tick_counter += 1;
                if self.wobjects[i].tick_counter >= weapon.damage_area_tick {
                    self.wobjects[i].tick_counter = 0;
                    let coil_x   = self.wobjects[i].pos.x.to_int();
                    let coil_y   = self.wobjects[i].pos.y.to_int();
                    let radius   = weapon.attract_radius.max(1);
                    let owner_i  = self.wobjects[i].owner_idx;
                    let dmg      = weapon.hit_damage;
                    let n_worms  = self.worms.len();
                    for wi in 0..n_worms {
                        if !self.worms[wi].alive { continue; }
                        let wx   = self.worms[wi].pos.x.to_int();
                        let wy   = self.worms[wi].pos.y.to_int();
                        let dist = crate::math::vector_length(wx - coil_x, wy - coil_y);
                        if dist < radius {
                            self.do_damage(wi, dmg, owner_i as i32);
                        }
                    }
                }
            }

            // ── 4. Worm hit detection ─────────────────────────────────────────
            if !do_explode && !do_remove
                && (weapon.hit_damage > 0 || weapon.blow_away > 0
                    || weapon.blood_on_hit > 0 || weapon.worm_collide)
            {
                let owner_idx = self.wobjects[i].owner_idx;
                let n_worms   = self.worms.len();
                'worm_loop: for wi in 0..n_worms {
                    if !self.worms[wi].alive { continue; }
                    if wi == owner_idx { continue; } // never self-hit (unlike C++ sprite check we use AABB)
                    let wix = self.worms[wi].pos.x.to_int();
                    let wiy = self.worms[wi].pos.y.to_int();
                    // Hit box: worm sprite is 16×16, nominal centre offset (+7, +5).
                    let dd  = weapon.detect_distance.max(1);
                    if (ipos_x - (wix + 7)).abs() < dd + 8
                        && (ipos_y - (wiy + 5)).abs() < dd + 8
                    {
                        // Knockback.
                        if weapon.blow_away > 0 {
                            let vx = Fixed(vel.x.0 * weapon.blow_away / 100);
                            let vy = Fixed(vel.y.0 * weapon.blow_away / 100);
                            let w  = &mut self.worms[wi];
                            w.vel.x.0 += vx.0;
                            w.vel.y.0 += vy.0;
                        }
                        // Damage (use scaled_damage override for charge weapons).
                        let hit_dmg = if self.wobjects[i].scaled_damage > 0 {
                            self.wobjects[i].scaled_damage
                        } else {
                            weapon.hit_damage
                        };
                        self.do_damage(wi, hit_dmg, owner_idx as i32);

                        // Swap Gun: exchange positions of owner and hit worm.
                        if weapon.worm_swap && owner_idx < self.worms.len() {
                            let owner_pos = self.worms[owner_idx].pos;
                            self.worms[owner_idx].pos = self.worms[wi].pos;
                            self.worms[wi].pos        = owner_pos;
                            self.worms[owner_idx].vel = FixedVec::ZERO;
                            self.worms[wi].vel        = FixedVec::ZERO;
                            do_remove = true;
                            break 'worm_loop;
                        }

                        // Blood particles from hit.
                        if weapon.blood_on_hit > 0 {
                            let blood_nobj_idx = self.tc.data.types.nobjects.iter()
                                .position(|n| n == "blood").unwrap_or(6);
                            let wpos = self.worms[wi].pos;
                            for _ in 0..weapon.blood_on_hit {
                                let angle  = self.rand.rand(128) as usize;
                                let vel_in = FixedVec { x: Fixed(vel.x.0 / 3), y: Fixed(vel.y.0 / 3) };
                                self.create_nobject2(blood_nobj_idx, angle, vel_in, wpos);
                            }
                        }

                        if weapon.worm_explode {
                            do_explode = true;
                            break 'worm_loop;
                        } else if weapon.boomerang_return_force > 0 {
                            // Boomerang passes through worms — damages but keeps flying.
                        } else {
                            do_remove = true;
                            break 'worm_loop;
                        }
                    }
                }
            }

            // ── 4.5. collide_with_objects: push nearby wobjects and nobjects ──
            if weapon.collide_with_objects && weapon.blow_away > 0 {
                let proj_pos = self.wobjects[i].pos;
                let proj_vel = self.wobjects[i].vel;
                let impulse_x = Fixed(proj_vel.x.0 * weapon.blow_away / 100);
                let impulse_y = Fixed(proj_vel.y.0 * weapon.blow_away / 100);
                // Push other wobjects within 2px.
                for j in 0..self.wobjects.len() {
                    if i == j { continue; }
                    let dx = (self.wobjects[j].pos.x.to_int() - proj_pos.x.to_int()).abs();
                    let dy = (self.wobjects[j].pos.y.to_int() - proj_pos.y.to_int()).abs();
                    if dx <= 2 && dy <= 2 {
                        self.wobjects[j].vel.x.0 += impulse_x.0;
                        self.wobjects[j].vel.y.0 += impulse_y.0;
                    }
                }
                // Push nobjects within 2px.
                for j in 0..self.nobjects.len() {
                    let dx = (self.nobjects[j].pos.x.to_int() - proj_pos.x.to_int()).abs();
                    let dy = (self.nobjects[j].pos.y.to_int() - proj_pos.y.to_int()).abs();
                    if dx <= 2 && dy <= 2 {
                        self.nobjects[j].vel.x.0 += impulse_x.0;
                        self.nobjects[j].vel.y.0 += impulse_y.0;
                    }
                }
            }

            // ── 5. Timeout ────────────────────────────────────────────────────
            if !do_explode && !do_remove && weapon.time_to_explo > 0 {
                self.wobjects[i].time_left -= 1;
                if self.wobjects[i].time_left < 0 {
                    // Check for on_expire_teleport.
                    if weapon.on_expire_teleport {
                        let owner = self.wobjects[i].owner_idx;
                        if owner < self.worms.len() && self.worms[owner].alive {
                            let teleport_pos = self.wobjects[i].pos;
                            self.worms[owner].pos = teleport_pos;
                            self.worms[owner].vel = FixedVec::ZERO;
                            do_remove = true;
                        } else {
                            do_explode = true;
                        }
                    } else {
                        do_explode = true;
                    }
                }
            }

            // ── 5.5. STLaser: iterate up to 8 more steps per frame ──────────────
            if weapon.shot_type == 4 && !do_explode && !do_remove {
                let max_extra = 7usize; // 7 more = 8 total
                for _ in 0..max_extra {
                    let extra_vel = self.wobjects[i].vel;
                    self.wobjects[i].pos += extra_vel;
                    let ep = self.wobjects[i].pos;
                    let ex = ep.x.to_int();
                    let ey = ep.y.to_int();
                    // Check terrain and worm hits at each sub-step.
                    let hit_terrain = !self.level.inside(ex, ey)
                        || self.level.material(ex, ey, &self.tc_materials).dirt_rock();
                    if hit_terrain { do_explode = true; break; }
                    // Quick worm check.
                    let oi = self.wobjects[i].owner_idx;
                    let dd = weapon.detect_distance.max(1);
                    let mut laser_hit = false;
                    for wi2 in 0..self.worms.len() {
                        if wi2 == oi || !self.worms[wi2].alive { continue; }
                        let wx2 = self.worms[wi2].pos.x.to_int();
                        let wy2 = self.worms[wi2].pos.y.to_int();
                        if (ex - (wx2+7)).abs() < dd+8 && (ey - (wy2+5)).abs() < dd+8 {
                            self.do_damage(wi2, weapon.hit_damage, oi as i32);
                            do_explode = weapon.worm_explode;
                            do_remove = !weapon.worm_explode;
                            laser_hit = true;
                            break;
                        }
                    }
                    if laser_hit { break; }
                }
            }

            // ── 6. Explode / remove ───────────────────────────────────────────
            if do_explode {
                let exp_x      = self.wobjects[i].pos.x.to_int();
                let exp_y      = self.wobjects[i].pos.y.to_int();
                let owner_idx = self.wobjects[i].owner_idx;

                // Create explosion sobject (applies damage, terrain destruction, splinters).
                if let Some(ref exp_name) = weapon.create_on_exp.clone() {
                    let sobj_idx = self.tc.data.types.sobjects.iter().position(|n| n == exp_name);
                    if let Some(idx) = sobj_idx {
                        self.consume_sobject_rand_by_idx(idx, exp_x, exp_y);
                    }
                }

                // Direct dirt effect (weapon-level, not sobject).
                if weapon.dirt_effect >= 0 {
                    let de     = weapon.dirt_effect as usize;
                    let rframe = self.tc.data.constants.textures.get(de).map(|t| t.rframe as u32).unwrap_or(0);
                    let rand_r = self.rand.rand(rframe);
                    if weapon.dirt_deposit {
                        // Deposit (fill) terrain instead of removing it.
                        self.apply_dirt_deposit(de, exp_x - 7, exp_y - 7, rand_r);
                        // DirtWar: credit the owner.
                        if matches!(self.mode, GameMode::DirtWar { .. }) && owner_idx < 4 {
                            self.dirt_scores[owner_idx] += 1;
                        }
                    } else {
                        self.apply_dirt_effect(de, exp_x - 7, exp_y - 7, rand_r);
                    }
                }

                // Splinters.
                if weapon.splinter_amount > 0 {
                    if let Some(ref stype_name) = weapon.splinter_type.clone() {
                        let sidx = self.tc.data.types.nobjects.iter().position(|n| n == stype_name);
                        if let Some(sidx) = sidx {
                            let spos = self.wobjects[i].pos;
                            let svel = self.wobjects[i].vel;
                            for _ in 0..weapon.splinter_amount {
                                let color_sub = self.rand.rand(2);
                                let _color    = weapon.splinter_colour - color_sub as i32;

                                // Splinter scatter mode: if splinter_scatter != 0, use velocity-based;
                                // otherwise, use angular distribution (create_nobject2 with random angle).
                                if weapon.splinter_scatter != 0 {
                                    // Velocity-based: splinter velocity = projectile velocity / splinter_scatter
                                    let splinter_vel = FixedVec {
                                        x: Fixed(svel.x.0 / weapon.splinter_scatter),
                                        y: Fixed(svel.y.0 / weapon.splinter_scatter),
                                    };
                                    // Use angle 0 (direction is determined by the splinter velocity)
                                    self.create_nobject2(sidx, 0, splinter_vel, spos);
                                } else {
                                    // Angular distribution: random angle, no initial velocity
                                    let angle = self.rand.rand(128) as usize;
                                    self.create_nobject2(sidx, angle, FixedVec::ZERO, spos);
                                }
                            }
                        }
                    }
                }

                // Chain explosion: detonate nearby wobjects if chain_explosion is true.
                if weapon.chain_explosion {
                    let exp_x = self.wobjects[i].pos.x.to_int();
                    let exp_y = self.wobjects[i].pos.y.to_int();
                    let det_range = weapon.detect_distance;

                    // Iterate through wobjects and detonate those within range.
                    let mut j = 0;
                    while j < self.wobjects.len() {
                        if j != i { // Don't detonate the same wobject
                            let wx = self.wobjects[j].pos.x.to_int();
                            let wy = self.wobjects[j].pos.y.to_int();
                            // Check if wobject is within detect_distance
                            if (wx - exp_x).abs() <= det_range && (wy - exp_y).abs() <= det_range {
                                self.wobjects[j].time_left = 0;
                            }
                        }
                        j += 1;
                    }
                }

                self.wobjects.remove(i);
                // Don't advance i.
            } else if do_remove {
                self.wobjects.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Create and push a WObject fired by worm `owner_idx` using `weapon_idx`.
    ///
    /// `angle` = 0-127 COSSIN table index; `extra_vel` = worm velocity contribution.
    fn create_wobject(&mut self, weapon_idx: usize, owner_idx: usize, angle: usize, extra_vel: FixedVec) {
        let Some(weapon) = self.tc.weapons.get(weapon_idx) else { return; };
        let weapon = weapon.clone();

        let speed  = weapon.speed as i64;
        let (cx, cy) = COSSIN_TABLE[angle & 0x7f];
        let base_vel = FixedVec {
            x: Fixed(((cx as i64 * speed) / 100) as i32),
            y: Fixed(((cy as i64 * speed) / 100) as i32),
        };

        // Worm velocity contribution (affectByWorm).
        let proj_vel = if weapon.affect_by_worm {
            FixedVec {
                x: Fixed(base_vel.x.0 + extra_vel.x.0),
                y: Fixed(base_vel.y.0 + extra_vel.y.0),
            }
        } else {
            base_vel
        };

        // Firing position: detect_distance + 5 pixels ahead of worm, 1 pixel up.
        // cx/cy are Q16.16 unit vectors; multiplying by integer pixels gives Q16.16 offset.
        // Mirrors C++: cossinTable[angle] * (detectDistance + 5) + pos - fixedvec(0, itof(1))
        let wpos = self.worms[owner_idx].pos;
        let fire_dist = weapon.detect_distance + 5;
        let fire_pos = FixedVec {
            x: Fixed(wpos.x.0 + (cx as i64 * fire_dist as i64) as i32),
            y: Fixed(wpos.y.0 + (cy as i64 * fire_dist as i64) as i32 - (1 << 16)),
        };

        // time_left = timeToExplo - rand(timeToExploV).
        let time_left = if weapon.time_to_explo > 0 {
            weapon.time_to_explo - self.rand.rand(weapon.time_to_explo_v.max(1) as u32) as i32
        } else {
            0
        };

        // curFrame initialization — mirrors C++ Weapon::fire() switch on shotType.
        let cur_frame_init = if weapon.start_frame >= 0 {
            use liero_data::weapon::shot_type;
            match weapon.shot_type {
                shot_type::NORMAL => {
                    if weapon.loop_anim {
                        if weapon.num_frames > 0 {
                            self.rand.rand((weapon.num_frames + 1) as u32) as i32
                        } else {
                            self.rand.rand(2) as i32
                        }
                    } else { 0 }
                }
                shot_type::D_TYPE_1 => ((angle as i32 - 12) >> 3).clamp(0, 12),
                // STSteerable(2), STDType2(3), STHoming(5): curFrame = angle (used as direction)
                _ => angle as i32,
            }
        } else {
            // Pixel-mode: cur_frame is colour index; C++ uses colorBullets - rand(2).
            weapon.color_bullets - self.rand.rand(2) as i32
        };

        let mut wobj = WObject::new(weapon_idx, owner_idx, fire_pos, proj_vel, time_left);
        wobj.cur_frame = cur_frame_init;
        self.wobjects.push(wobj);
    }

    /// Like `create_wobject` but scales speed and hit_damage by `scale_pct` / 100.
    ///
    /// Used by charge-up weapons (Gauss Sniper): stage 0 = 25%, full = 100%.
    fn create_wobject_charged(
        &mut self,
        weapon_idx: usize,
        owner_idx:  usize,
        angle:      usize,
        extra_vel:  FixedVec,
        scale_pct:  i32,
    ) {
        let Some(weapon) = self.tc.weapons.get(weapon_idx) else { return; };
        let mut weapon = weapon.clone();

        // Scale speed and damage by charge percentage.
        weapon.speed      = weapon.speed      * scale_pct / 100;
        weapon.hit_damage = weapon.hit_damage * scale_pct / 100;

        let speed = weapon.speed as i64;
        let (cx, cy) = COSSIN_TABLE[angle & 0x7f];
        let base_vel = FixedVec {
            x: Fixed(((cx as i64 * speed) / 100) as i32),
            y: Fixed(((cy as i64 * speed) / 100) as i32),
        };
        let proj_vel = if weapon.affect_by_worm {
            let s = speed.max(100);
            FixedVec {
                x: Fixed(base_vel.x.0 + (extra_vel.x.0 as i64 * 100 / s) as i32),
                y: Fixed(base_vel.y.0 + (extra_vel.y.0 as i64 * 100 / s) as i32),
            }
        } else {
            base_vel
        };

        let wpos = self.worms[owner_idx].pos;
        let fire_dist = weapon.detect_distance + 5;
        let fire_pos = FixedVec {
            x: Fixed(wpos.x.0 + (cx as i64 * fire_dist as i64) as i32),
            y: Fixed(wpos.y.0 + (cy as i64 * fire_dist as i64) as i32 - (1 << 16)),
        };
        let time_left = if weapon.time_to_explo > 0 {
            weapon.time_to_explo - self.rand.rand(weapon.time_to_explo_v.max(1) as u32) as i32
        } else {
            0
        };

        let cur_frame_init = if weapon.start_frame >= 0 {
            use liero_data::weapon::shot_type;
            match weapon.shot_type {
                shot_type::NORMAL => {
                    if weapon.loop_anim {
                        if weapon.num_frames > 0 {
                            self.rand.rand((weapon.num_frames + 1) as u32) as i32
                        } else {
                            self.rand.rand(2) as i32
                        }
                    } else { 0 }
                }
                shot_type::D_TYPE_1 => ((angle as i32 - 12) >> 3).clamp(0, 12),
                _ => angle as i32,
            }
        } else {
            weapon.color_bullets - self.rand.rand(2) as i32
        };

        let mut wobj = WObject::new(weapon_idx, owner_idx, fire_pos, proj_vel, time_left);
        wobj.cur_frame = cur_frame_init;
        // Store the scaled damage; collision code reads this instead of weapon.hit_damage.
        wobj.scaled_damage = weapon.hit_damage.max(0);
        self.wobjects.push(wobj);
    }

    /// Spawn a shell casing nobject after weapon fire.
    /// Shells eject perpendicular to the fire angle with small random velocity.
    fn spawn_shell_casing(&mut self, worm_idx: usize, fire_angle: usize) {
        // Find shell nobject type by name, or use hardcoded index (C++ uses index 7: shells).
        let shell_idx = self.tc.data.types.nobjects.iter()
            .position(|name| name.eq_ignore_ascii_case("shells"))
            .unwrap_or(7); // Fallback to index 7 if "shells" not found

        if shell_idx >= self.tc.nobjects.len() {
            return; // Shell nobject type doesn't exist in TC
        }

        let worm_pos = self.worms[worm_idx].pos;

        // Shell ejects at angle perpendicular to fire angle (90 degrees offset).
        // C++ code: shell velocity has random X and upward Y component.
        // We'll use the opposite direction (180 degrees from fire angle) for shell ejection.
        let shell_angle = (fire_angle as i32 + 64).rem_euclid(128) as usize;

        // Small random velocities as per C++ code: velY = -rand(20000), velX = rand(16000) - 8000
        let vel_y = Fixed(-((self.rand.rand(20000) as i32)));
        let vel_x = Fixed((self.rand.rand(16000) as i32) - 8000);
        let shell_vel = FixedVec { x: vel_x, y: vel_y };

        // Create shell at worm position
        self.create_nobject2(shell_idx, shell_angle, shell_vel, worm_pos);
    }

    /// Returns `true` if all 5×5 pixels centred on `(x, y)` are background.
    fn check_bonus_spawn_position(level: &Level, tc_mat: &[Material], x: i32, y: i32) -> bool {
        let x1 = (x - 2).max(0);
        let y1 = (y - 2).max(0);
        let x2 = (x + 3).min(level.width  as i32);
        let y2 = (y + 3).min(level.height as i32);

        for cx in x1..x2 {
            for cy in y1..y2 {
                if level.material(cx, cy, tc_mat).dirt_rock() {
                    return false;
                }
            }
        }
        true
    }

    /// Port of `Game::createBonus()` in `src/game/game.cpp`.
    ///
    /// Consumes rand for position attempts and bonus attributes.
    /// Must be called with the same RNG state as C++ to preserve determinism.
    fn create_bonus(&mut self) {
        if self.bonuses.iter().filter(|b| b.is_some()).count() >= self.max_bonuses as usize {
            return;
        }

        let rw       = self.tc.data.constants.bonus_spawn_rect_w as u32;
        let rh       = self.tc.data.constants.bonus_spawn_rect_h as u32;
        let rx       = self.tc.data.constants.bonus_spawn_rect_x;
        let ry       = self.tc.data.constants.bonus_spawn_rect_y;
        let use_rect = self.tc.data.hacks.bonus_spawn_rect;
        let only_health = self.tc.data.hacks.bonus_only_health;
        let only_weapon = self.tc.data.hacks.bonus_only_weapon;

        for _ in 0..50000usize {
            let mut ix = self.rand.rand(rw) as i32;
            let mut iy = self.rand.rand(rh) as i32;

            if use_rect {
                ix += rx;
                iy += ry;
            }

            if Self::check_bonus_spawn_position(&self.level, &self.tc_materials, ix, iy) {
                // frame: 0=weapon, 1=health, 2=sprint_boots, 3=kevlar, 4=double_jump.
                // Distribution: 40% weapon, 40% health, 20% power-up (split 3 ways).
                let frame = if only_health {
                    1
                } else if only_weapon {
                    0
                } else {
                    let r = self.rand.rand(5) as i32; // 0-4
                    match r {
                        0 | 1 => 0,      // 40% weapon
                        2 | 3 => 1,      // 40% health
                        4     => 2 + self.rand.rand(3) as i32, // 20% power-up (2/3/4)
                        _     => 0,
                    }
                };

                // Lookup timer range from TcBonus[frame].
                let (timer_base, timer_v) = self.tc.data.constants
                    .bonuses
                    .get(frame as usize)
                    .map(|b| (b.timer, b.timer_v))
                    .unwrap_or((3000, 2000));

                let timer = self.rand.rand(timer_v as u32) as i32 + timer_base;

                // Weapon selection (for frame == 0).
                let weapon = if frame == 0 {
                    let n_weapons = self.tc.data.types.weapons.len() as u32;
                    // C++: do { weapon = rand(weapons.size()); } while(weapTable[weapon]==2)
                    // Default settings have all weapons enabled (weapTable[i] == 0 for all i).
                    self.rand.rand(n_weapons) as i32
                } else {
                    0
                };

                let _live_count = self.bonuses.iter().filter(|b| b.is_some()).count();

                let new_bonus = Bonus {
                    x:     ix << 16, // itof
                    y:     iy << 16, // itof
                    vel_y: 0,
                    frame,
                    timer,
                    weapon,
                    used:  false,
                };
                // Find first None slot (pool allocation — lowest free index, mirrors C++ ExactObjectList::getFreeObject)
                if let Some(slot) = self.bonuses.iter().position(|b| b.is_none()) {
                    self.bonuses[slot] = Some(new_bonus);
                } else {
                    self.bonuses.push(Some(new_bonus));
                }

                // C++ also calls sobjectTypes[7].create() here (teleport_flash).
                // teleport_flash has startSound=-1 and damage=0, so it consumes
                // zero rand — safe to skip in headless mode.

                return;
            }
        }
        // 50 000 attempts exhausted — no bonus spawned (no rand consumed beyond the loop).
    }

    // ── Respawn helpers ───────────────────────────────────────────────────

    /// Compute the centroid (integer pixels) of all worms except `exclude_idx`.
    fn enemy_centroid(worms: &[Worm], exclude_idx: usize) -> (i32, i32) {
        let mut sx = 0i64; let mut sy = 0i64; let mut n = 0i64;
        for (i, w) in worms.iter().enumerate() {
            if i == exclude_idx { continue; }
            sx += w.pos.x.to_int() as i64;
            sy += w.pos.y.to_int() as i64;
            n += 1;
        }
        if n == 0 { (0, 0) } else { ((sx / n) as i32, (sy / n) as i32) }
    }

    /// Port of C++ `Worm::beginRespawn`.
    /// Picks a spawn position using `rand` and `checkRespawnPosition`.
    #[allow(clippy::too_many_arguments)]
    fn begin_respawn(
        worm:      &mut Worm,
        level:     &Level,
        tc_mat:    &[Material],
        consts:    &liero_data::tc::TcConstants,
        rand:      &mut Mwc,
        enemy_x: i32, enemy_y: i32,
        old_x: i32, old_y: i32,
    ) {
        use crate::fixed::Fixed;

        // logicRespawn = (old_x - 80, old_y - 80) — visual anchor for the
        // "worm zooms into position" animation. Clamped in do_respawning.
        worm.logic_respawn_x = old_x - 80;
        worm.logic_respawn_y = old_y - 80;

        let rx = consts.worm_spawn_rect_x;
        let ry = consts.worm_spawn_rect_y;
        let rw = consts.worm_spawn_rect_w;
        let rh = consts.worm_spawn_rect_h;

        let mut trials = 0u32;
        loop {
            let px = rx + rand.rand(rw as u32) as i32;
            let py = ry + rand.rand(rh as u32) as i32;

            // Gravity settle: drop down until terrain below.
            let mut sy = py;
            while sy + 4 < level.height as i32
                && level.material(px, sy + 4, tc_mat).background()
            {
                sy += 1;
            }

            worm.pos.x = Fixed::from_int(px);
            worm.pos.y = Fixed::from_int(sy);

            trials += 1;
            let valid = Self::check_respawn_position(
                level, tc_mat, consts,
                enemy_x, enemy_y, old_x, old_y - sy,
                px, sy,
            );

            if trials >= 50000 || valid {
                break;
            }
        }

        worm.killed_timer = -1;
    }

    /// Port of C++ `checkRespawnPosition`.
    fn check_respawn_position(
        level:     &Level,
        tc_mat:    &[Material],
        consts:    &liero_data::tc::TcConstants,
        enemy_x: i32, enemy_y: i32,
        old_x: i32, delta_y: i32,   // oldX unused as deltaX, deltaY = old - new_y
        x: i32, y: i32,
    ) -> bool {
        // C++ uses deltaX = oldX (never subtracted?), deltaY = oldY - y.
        let delta_x = old_x; // mirrors: int deltaX = oldX (C++ bug/feature)
        let enemy_dx = enemy_x - x;
        let enemy_dy = enemy_y - y;

        let last = consts.worm_min_spawn_dist_last;
        let enemy = consts.worm_min_spawn_dist_enemy;

        if (delta_x.abs() <= last && delta_y.abs() <= last)
            || (enemy_dx.abs() <= enemy && enemy_dy.abs() <= enemy)
        {
            return false;
        }

        // Check 7×9 bounding box for rock.
        let max_x = (x + 3).min(level.width  as i32 - 1);
        let max_y = (y + 4).min(level.height as i32 - 1);
        let min_x = (x - 3).max(0);
        let min_y = (y - 4).max(0);

        let mut ix = min_x;
        while ix != max_x {
            let mut jy = min_y;
            while jy != max_y {
                if level.material(ix, jy, tc_mat).rock() { return false; }
                jy += 1;
            }
            ix += 1;
        }
        true
    }

    /// Port of C++ `Worm::doRespawning` (animation frame, no rand).
    /// Port of `Worm::doRespawning` in `src/game/worm.cpp`.
    ///
    /// Slides the logic-respawn position toward the target, then — when it
    /// converges to within 5 pixels on both axes AND `ready == true` —
    /// completes the spawn:
    ///
    /// 1. Consumes `rand(tex0_rframe)` to mirror `drawDirtEffect` (always 1 rand call).
    /// 2. Consumes `rand.next()` to pick initial facing direction (mirrors `game.rand() & 1`).
    /// 3. Sets `aiming_angle` to `itof(32)` or `itof(96)` based on that bit.
    /// 4. Zeroes velocity, sets `alive = true`, clears `ready`.
    fn do_respawning(worm: &mut Worm, level: &Level, rand: &mut crate::rand::Mwc, tex0_rframe: u32) {
        let max_x = level.width  as i32 - 158;
        let max_y = level.height as i32 - 158;

        let target_x = worm.pos.x.to_int() - 80;
        let target_y = worm.pos.y.to_int() - 80;

        for _ in 0..4 {
            if worm.logic_respawn_x < target_x { worm.logic_respawn_x += 1; }
            else if worm.logic_respawn_x > target_x { worm.logic_respawn_x -= 1; }

            if worm.logic_respawn_y < target_y { worm.logic_respawn_y += 1; }
            else if worm.logic_respawn_y > target_y { worm.logic_respawn_y -= 1; }
        }

        // Clamp logic respawn coords (mirrors C++ limitXY).
        worm.logic_respawn_x = worm.logic_respawn_x.clamp(0, max_x);
        worm.logic_respawn_y = worm.logic_respawn_y.clamp(0, max_y);

        let dest_x = target_x.clamp(0, max_x);
        let dest_y = target_y.clamp(0, max_y);

        // Check convergence + ready flag — mirrors C++ doRespawning spawn condition.
        if worm.logic_respawn_x < dest_x + 5
            && worm.logic_respawn_x > dest_x - 5
            && worm.logic_respawn_y < dest_y + 5
            && worm.logic_respawn_y > dest_y - 5
            && worm.ready
        {
            // 1. drawDirtEffect always calls rand(tex.rFrame) once.
            rand.rand(tex0_rframe);

            // 2. Direction selection: game.rand() & 1  (no modulo — raw next()).
            let dir_raw = rand.next();
            if dir_raw & 1 != 0 {
                worm.aiming_angle = 32 << 16; // itof(32)
            } else {
                worm.aiming_angle = 96 << 16; // itof(96)
            }

            // 3. Finalise spawn state — mirrors C++ doRespawning body.
            worm.vel = crate::fixed::FixedVec::default();
            worm.alive = true;
            worm.ready = false;
        }
    }

    /// Port of `Worm::calculateReactionForce` + `Worm::processPhysics`.
    ///
    /// Collision probe points (x,y offsets from worm pixel position):
    /// - DOWN (reaction=0): 3 points at y−4 (above)
    /// - LEFT (reaction=1): 7 points at x+1 (right side)
    /// - UP   (reaction=2): 3 points at y+4 (below)
    /// - RIGHT(reaction=3): 7 points at x−1 (left side)
    fn step_worm_physics(
        worm: &mut Worm,
        level: &Level,
        tc_materials: &[Material],
        consts: &liero_data::tc::TcConstants,
        fall_damage_enabled: bool,
    ) {
        // Collision probe tables — mirrors C++ `colPoints[4][7]`.
        // Each entry is (dx, dy) relative to the integer worm position.
        const COL_POINTS: [[(i32, i32); 7]; 4] = [
            // DOWN (index 0): 3 points probing y−4 (detect ceiling)
            [(-1,-4),(0,-4),(1,-4),(0,0),(0,0),(0,0),(0,0)],
            // LEFT (index 1): 7 points probing x+1 (detect right wall)
            [(1,-3),(1,-2),(1,-1),(1,0),(1,1),(1,2),(1,3)],
            // UP (index 2): 3 points probing y+4 (detect floor)
            [(-1,4),(0,4),(1,4),(0,0),(0,0),(0,0),(0,0)],
            // RIGHT (index 3): 7 points probing x−1 (detect left wall)
            [(-1,-3),(-1,-2),(-1,-1),(-1,0),(-1,1),(-1,2),(-1,3)],
        ];
        const COL_POINT_COUNT: [usize; 4] = [3, 7, 3, 7];

        // Compute next position (integer pixel coords).
        let next_x = (worm.pos.x + worm.vel.x).to_int();
        let next_y = (worm.pos.y + worm.vel.y).to_int();

        // ── calculateReactionForce (4 directions) ──────────────────────────
        // Mirrors C++ worm.process() inner loop: called 4 times, then edge checks.
        for dir in 0..4usize {
            worm.reacts[dir] = 0;
            for i in 0..COL_POINT_COUNT[dir] {
                let (dx, dy) = COL_POINTS[dir][i];
                let cx = next_x + dx;
                let cy = next_y + dy;
                let mat = level.material_wrap(cx, cy, tc_materials);
                if !mat.background() {
                    worm.reacts[dir] += 1;
                }
            }

            // Level boundary reactions (applied in every iteration, same as C++).
            if next_x < 4 {
                worm.reacts[react::RIGHT] += 5;
            } else if next_x > (level.width as i32) - 5 {
                worm.reacts[react::LEFT] += 5;
            }

            if next_y < 5 {
                worm.reacts[react::DOWN] += 5;
            } else if next_y > (level.height as i32) - 6 {
                worm.reacts[react::UP] += 5;
            }
        }

        // Stepping-up/down assist (mirrors the block after main loop in C++).
        if worm.reacts[react::DOWN] < 2
            && worm.reacts[react::UP] > 0
            && (worm.reacts[react::LEFT] > 0 || worm.reacts[react::RIGHT] > 0)
        {
            // Ledge step-up: move pos up 1 pixel, recalculate LEFT/RIGHT.
            worm.pos.y -= Fixed::from_int(1);
            let ny = (worm.pos.y + worm.vel.y).to_int();
            for dir in [react::LEFT, react::RIGHT] {
                worm.reacts[dir] = 0;
                for i in 0..COL_POINT_COUNT[dir] {
                    let (dx, dy) = COL_POINTS[dir][i];
                    let mat = level.material_wrap(next_x + dx, ny + dy, tc_materials);
                    if !mat.background() { worm.reacts[dir] += 1; }
                }
            }
        }

        if worm.reacts[react::UP] < 2
            && worm.reacts[react::DOWN] > 0
            && (worm.reacts[react::LEFT] > 0 || worm.reacts[react::RIGHT] > 0)
        {
            // Ledge step-down: move pos down 1 pixel, recalculate LEFT/RIGHT.
            worm.pos.y += Fixed::from_int(1);
            let ny = (worm.pos.y + worm.vel.y).to_int();
            for dir in [react::LEFT, react::RIGHT] {
                worm.reacts[dir] = 0;
                for i in 0..COL_POINT_COUNT[dir] {
                    let (dx, dy) = COL_POINTS[dir][i];
                    let mat = level.material_wrap(next_x + dx, ny + dy, tc_materials);
                    if !mat.background() { worm.reacts[dir] += 1; }
                }
            }
        }

        // ── processPhysics ────────────────────────────────────────────────
        // Friction when standing on ground (RFUp > 0).
        if worm.reacts[react::UP] > 0 {
            worm.vel.x = Fixed(
                (worm.vel.x.0 as i64 * consts.worm_fric_mult as i64
                    / consts.worm_fric_div as i64) as i32
            );
        }

        let abs_vel_x = Fixed(worm.vel.x.0.wrapping_abs());
        let abs_vel_y = Fixed(worm.vel.y.0.wrapping_abs());

        // Horizontal bounce / stop.
        let rh = if worm.vel.x.0 >= 0 { worm.reacts[react::LEFT]  }
                 else                  { worm.reacts[react::RIGHT] };
        let mbh = if worm.vel.x.0 > 0 {  consts.min_bounce_right }
                  else                 { -consts.min_bounce_left  };

        if worm.vel.x.0 != 0 && rh > 0 {
            if abs_vel_x.0 > mbh {
                worm.vel.x = Fixed((-worm.vel.x.0).wrapping_div(3));
            } else {
                worm.vel.x = Fixed(0);
            }
        }

        // Vertical bounce / stop.
        let rv = if worm.vel.y.0 >= 0 { worm.reacts[react::UP]   }
                 else                  { worm.reacts[react::DOWN] };
        let mbv = if worm.vel.y.0 > 0 {  consts.min_bounce_down }
                  else                 { -consts.min_bounce_up   };

        if worm.vel.y.0 != 0 && rv > 0 {
            if abs_vel_y.0 > mbv {
                worm.vel.y = Fixed((-worm.vel.y.0).wrapping_div(3));
            } else {
                worm.vel.y = Fixed(0);
            }
        }

        // Fall damage on bounce (mirrors C++ worm.cpp:169-184).
        // Only apply damage if fall_damage is enabled (TC flag).
        if fall_damage_enabled {
            // Horizontal bounce damage.
            if worm.vel.x.0 != 0 && rh > 0 && abs_vel_x.0 > mbh {
                let fall_dmg = if worm.vel.x.0 > 0 {
                    consts.fall_damage_right
                } else {
                    consts.fall_damage_left
                };
                worm.health = worm.health.saturating_sub(fall_dmg);
            }

            // Vertical bounce damage.
            if worm.vel.y.0 != 0 && rv > 0 && abs_vel_y.0 > mbv {
                let fall_dmg = if worm.vel.y.0 > 0 {
                    consts.fall_damage_down
                } else {
                    consts.fall_damage_up
                };
                worm.health = worm.health.saturating_sub(fall_dmg);
            }
        }

        // Gravity (when not standing on ground).
        if worm.reacts[react::UP] == 0 {
            worm.vel.y += Fixed(consts.worm_gravity);
        }

        // ── Worm float: reduce gravity when above float level ────────────────────
        // If enabled (worm_float_power > 0) and worm Y is above the float level,
        // apply an upward force to counteract gravity.
        let worm_y_px = worm.pos.y.to_int();
        if consts.worm_float_power != 0 && worm_y_px < consts.worm_float_level {
            worm.vel.y.0 -= consts.worm_float_power;
        }

        // Apply velocity (blocked by 2+ reactions in movement direction).
        if worm.reacts[if worm.vel.x.0 >= 0 { react::LEFT  } else { react::RIGHT }] < 2 {
            worm.pos.x += worm.vel.x;
        }
        if worm.reacts[if worm.vel.y.0 >= 0 { react::UP   } else { react::DOWN  }] < 2 {
            worm.pos.y += worm.vel.y;
        }
    }

    /// FNV-1a 64-bit checksum over simulation state.
    ///
    /// Matches C++ `fullGameChecksum()` in `src/game/game.cpp:947-999`.
    ///
    /// # Algorithm
    /// Word-at-a-time FNV-1a: each `mix(v: u32)` does `h ^= v as u64; h *= PRIME`.
    ///
    /// # Field order
    /// 1. `rand.x`
    /// 2. `cycles`
    /// 3. Per worm (in order): `pos.x, pos.y, vel.x, vel.y, aiming_angle,
    ///    health, lives, kills, timer, current_weapon`
    /// 4. Per wobject: `pos.x, pos.y, vel.x, vel.y, time_left`
    /// 5. Per nobject: `pos.x, pos.y, vel.x, vel.y, time_left`
    pub fn checksum(&self) -> u64 {
        const FNV_OFFSET: u64 = 14695981039346656037;
        const FNV_PRIME:  u64 = 1099511628211;

        let mut h = FNV_OFFSET;

        // Inline closure — mirrors C++ lambda `mix`.
        macro_rules! mix {
            ($v:expr) => {{
                h ^= ($v as u32) as u64;
                h = h.wrapping_mul(FNV_PRIME);
            }};
        }

        // RNG state and frame counter.
        mix!(self.rand.x);
        mix!(self.cycles);

        // Per-worm state.
        for (_wi, w) in self.worms.iter().enumerate() {
            mix!(w.pos.x.0);
            mix!(w.pos.y.0);
            mix!(w.vel.x.0);
            mix!(w.vel.y.0);
            mix!(w.aiming_angle);
            mix!(w.health);
            mix!(w.lives);
            mix!(w.kills);
            mix!(w.timer);
            mix!(w.current_weapon);
            mix!(w.rope_active as i32);
            mix!(w.rope_attached as i32);
            mix!(w.rope_len);
            mix!(w.charge_ticks);
            mix!(w.fire_cone);
            mix!(w.killed_timer);
            for &ammo in &w.weapon_ammo { mix!(ammo); }
        }

        // Per-wobject state.
        for obj in &self.wobjects {
            mix!(obj.pos.x.0);
            mix!(obj.pos.y.0);
            mix!(obj.vel.x.0);
            mix!(obj.vel.y.0);
            mix!(obj.time_left);
        }

        // Per-nobject state.
        for (_ni, nobj) in self.nobjects.iter().enumerate() {
            mix!(nobj.pos.x.0);
            mix!(nobj.pos.y.0);
            mix!(nobj.vel.x.0);
            mix!(nobj.vel.y.0);
            mix!(nobj.time_left);
        }

        // Per-bonus state (x, y, velY, frame, weapon) — matches C++ fullGameChecksum.
        // Iterate in slot order, skipping None (pool semantics).
        for bonus in self.bonuses.iter().flatten() {
            mix!(bonus.x);
            mix!(bonus.y);
            mix!(bonus.vel_y);
            mix!(bonus.frame);
            mix!(bonus.weapon);
        }

        // Level terrain pixels — ensures desync detection if terrain is modified.
        for px in self.level.pixels() {
            mix!(*px as i32);
        }

        // Game mode state
        mix!(self.tag_worm.map(|i| i as i32).unwrap_or(-1));
        for &cp in &self.worm_checkpoint { mix!(cp); }
        for &laps in &self.worm_laps { mix!(laps); }
        mix!(self.round_end_timer);

        // KotH state
        for &ticks in &self.koth_ticks { mix!(ticks); }

        // BombTag state
        mix!(self.bomb_timer);

        // DirtWar state
        for &score in &self.dirt_scores { mix!(score); }

        // Pending result
        mix!(match &self.pending_result {
            None => -1i32,
            Some(GameResult::WormWins(i)) => *i as i32,
            Some(GameResult::TeamWins(t)) => 100 + *t,
            Some(GameResult::Draw) => 200,
        });

        // Configurable settings
        mix!(self.blood_pct);
        mix!(self.loading_time_factor);
        mix!(self.friendly_fire as i32);
        mix!(self.random_weapons as i32);

        h
    }

    // ── Sub-checksums for debug-desync tooling ────────────────────────────────

    /// FNV-1a checksum over worm positions, velocities, health, and alive state only.
    pub fn worm_checksum(&self) -> u64 {
        const FNV_PRIME: u64 = 1099511628211;
        let mut h: u64 = 14695981039346656037;
        macro_rules! mix { ($v:expr) => {{ h ^= ($v as u32) as u64; h = h.wrapping_mul(FNV_PRIME); }}; }
        for w in &self.worms {
            mix!(w.pos.x.0); mix!(w.pos.y.0);
            mix!(w.vel.x.0); mix!(w.vel.y.0);
            mix!(w.health);  mix!(w.alive as i32);
        }
        h
    }

    /// FNV-1a checksum over active wobject positions, velocities, and time_left.
    pub fn wobject_checksum(&self) -> u64 {
        const FNV_PRIME: u64 = 1099511628211;
        let mut h: u64 = 14695981039346656037;
        macro_rules! mix { ($v:expr) => {{ h ^= ($v as u32) as u64; h = h.wrapping_mul(FNV_PRIME); }}; }
        for wo in &self.wobjects {
            mix!(wo.pos.x.0); mix!(wo.pos.y.0);
            mix!(wo.vel.x.0); mix!(wo.vel.y.0);
            mix!(wo.time_left);
        }
        h
    }

    /// FNV-1a checksum over active nobject positions and velocities.
    pub fn nobject_checksum(&self) -> u64 {
        const FNV_PRIME: u64 = 1099511628211;
        let mut h: u64 = 14695981039346656037;
        macro_rules! mix { ($v:expr) => {{ h ^= ($v as u32) as u64; h = h.wrapping_mul(FNV_PRIME); }}; }
        for no in &self.nobjects {
            mix!(no.pos.x.0); mix!(no.pos.y.0);
            mix!(no.vel.x.0); mix!(no.vel.y.0);
        }
        h
    }

    // ── Rollback API ──────────────────────────────────────────────────────────

    /// Capture the current mutable state into a `GameSnapshot`.
    ///
    /// Clones worms, wobjects, nobjects, bonuses, and level pixels.
    /// `tc` and `tc_materials` are constant — not included.
    pub fn save_snapshot(&self) -> GameSnapshot {
        GameSnapshot {
            rand:           self.rand.clone(),
            cycles:         self.cycles,
            worms:          self.worms.clone(),
            wobjects:       self.wobjects.clone(),
            nobjects:       self.nobjects.clone(),
            sobjects:       self.sobjects.clone(),
            bonuses:        self.bonuses.clone(),
            floating_damages: self.floating_damages.clone(),
            level_pixels:   self.level.pixels().to_vec(),
            koth_ticks:     self.koth_ticks,
            bomb_timer:     self.bomb_timer,
            dirt_scores:    self.dirt_scores,
            tag_worm:       self.tag_worm,
            worm_checkpoint: self.worm_checkpoint,
            worm_laps:      self.worm_laps,
            round_end_timer: self.round_end_timer,
            pending_result: self.pending_result.clone(),
            screen_flash:   self.screen_flash,
            viewport_shake: self.viewport_shake,
        }
    }

    /// Overwrite mutable state from a previously saved `GameSnapshot`.
    ///
    /// The `tc` and `tc_materials` fields are left unchanged (they are constant).
    pub fn restore_snapshot(&mut self, snap: &GameSnapshot) {
        self.rand           = snap.rand.clone();
        self.cycles         = snap.cycles;
        self.worms          = snap.worms.clone();
        self.wobjects       = snap.wobjects.clone();
        self.nobjects       = snap.nobjects.clone();
        self.sobjects       = snap.sobjects.clone();
        self.bonuses        = snap.bonuses.clone();
        self.floating_damages = snap.floating_damages.clone();
        self.koth_ticks     = snap.koth_ticks;
        self.bomb_timer     = snap.bomb_timer;
        self.dirt_scores    = snap.dirt_scores;
        self.tag_worm       = snap.tag_worm;
        self.worm_checkpoint = snap.worm_checkpoint;
        self.worm_laps      = snap.worm_laps;
        self.round_end_timer = snap.round_end_timer;
        self.pending_result = snap.pending_result.clone();
        self.screen_flash   = snap.screen_flash;
        self.viewport_shake = snap.viewport_shake;
        self.level.restore_pixels(&snap.level_pixels);
        self.sound_events.clear();
    }

    // ── Game mode API ─────────────────────────────────────────────────────────

    /// Set the game mode and configure worm state accordingly.
    ///
    /// Call this after `Game::new()` and before the first `step()`.
    pub fn set_mode(&mut self, mode: GameMode) {
        match &mode {
            GameMode::TeamDeathmatch => {
                // Worms 0, 2 → team A; worms 1, 3 → team B.
                for (i, w) in self.worms.iter_mut().enumerate() {
                    w.team_id = (i % 2) as i32;
                }
            }
            GameMode::LastManStanding => {
                // One life each — no respawn after death.
                for w in &mut self.worms { w.lives = 1; }
            }
            GameMode::Juggernaut => {
                // Worm 0 starts as juggernaut with 3× HP.
                if let Some(w) = self.worms.first_mut() {
                    w.is_juggernaut = true;
                    w.health        = (self.worm_health * 3).min(999);
                }
            }
            GameMode::BombTag { fuse } => {
                // Worm 0 starts with the bomb; fuse starts ticking immediately.
                self.bomb_timer = *fuse;
                if let Some(w) = self.worms.first_mut() {
                    w.has_bomb = true;
                }
            }
            GameMode::KillRace { .. } => {
                // Infinite respawns — give worms many lives.
                for w in &mut self.worms { w.lives = 999; }
            }
            _ => {}
        }
        self.mode = mode;
    }

    /// Check whether the match has ended.
    ///
    /// Returns `Some(GameResult)` when a winner (or draw) is determined,
    /// `None` while the match is still in progress.
    ///
    /// Call once per frame after `step()`.
    /// Helper: register a game result, starting the round-end delay.
    fn register_result(&mut self, result: GameResult) {
        if self.pending_result.is_none() {
            self.pending_result = Some(result);
            self.round_end_timer = 180; // 3 seconds at 60 fps
        }
    }

    pub fn game_over(&mut self) -> Option<GameResult> {
        // If there's a pending result waiting for the round-end timer to expire, return it when ready.
        if let Some(_result) = &self.pending_result {
            if self.round_end_timer <= 0 {
                return self.pending_result.take();
            }
            // Timer still running, game not over yet.
            return None;
        }

        // Need at least two worms for a meaningful game.
        if self.worms.len() < 2 { return None; }

        let result_opt = match &self.mode {
            GameMode::LastManStanding => {
                // Active = not eliminated.
                let active: Vec<usize> = (0..self.worms.len())
                    .filter(|&i| !self.worms[i].eliminated)
                    .collect();
                match active.len() {
                    0 => Some(GameResult::Draw),
                    1 => Some(GameResult::WormWins(active[0])),
                    _ => None,
                }
            }

            GameMode::TeamDeathmatch => {
                // A team is still alive if any of its worms are not eliminated.
                let team_alive = |team: i32| -> bool {
                    self.worms.iter().any(|w| w.team_id == team && !w.eliminated)
                };
                let a_alive = team_alive(0);
                let b_alive = team_alive(1);
                match (a_alive, b_alive) {
                    (true,  false) => Some(GameResult::TeamWins(0)),
                    (false, true)  => Some(GameResult::TeamWins(1)),
                    (false, false) => Some(GameResult::Draw),
                    (true,  true)  => None,
                }
            }

            GameMode::KingOfHill { time_limit } => {
                // First worm to reach time_limit ticks wins.
                for (i, &ticks) in self.koth_ticks.iter().enumerate() {
                    if i < self.worms.len() && ticks >= *time_limit {
                        return Some(GameResult::WormWins(i));
                    }
                }
                None
            }

            GameMode::BombTag { .. } => {
                // Ends when only one worm is not eliminated.
                let active: Vec<usize> = (0..self.worms.len())
                    .filter(|&i| !self.worms[i].eliminated)
                    .collect();
                match active.len() {
                    0 => Some(GameResult::Draw),
                    1 => Some(GameResult::WormWins(active[0])),
                    _ => None,
                }
            }

            GameMode::ZombieMode => {
                // Count non-zombie (human) worms.
                let humans: Vec<usize> = (0..self.worms.len())
                    .filter(|&i| !self.worms[i].is_zombie)
                    .collect();
                match humans.len() {
                    0 => Some(GameResult::Draw),
                    1 => Some(GameResult::WormWins(humans[0])),
                    _ => None,
                }
            }

            GameMode::Juggernaut => {
                // Juggernaut keeps playing indefinitely — game ends when non-juggernauts
                // are all eliminated or only the juggernaut remains alive.
                let non_jug_active = self.worms.iter()
                    .filter(|w| !w.is_juggernaut && !w.eliminated)
                    .count();
                if non_jug_active == 0 {
                    // Juggernaut wins.
                    let winner = self.worms.iter()
                        .position(|w| w.is_juggernaut)
                        .unwrap_or(0);
                    Some(GameResult::WormWins(winner))
                } else {
                    None
                }
            }

            GameMode::DirtWar { time_limit } => {
                // Time's up — worm with the most dirt deposited wins.
                if self.cycles >= *time_limit {
                    let n = self.worms.len().min(4);
                    let winner = self.dirt_scores[..n]
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, &s)| s)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    Some(GameResult::WormWins(winner))
                } else {
                    None
                }
            }

            GameMode::KillRace { frag_limit } => {
                for (i, w) in self.worms.iter().enumerate() {
                    if w.kills >= *frag_limit {
                        return Some(GameResult::WormWins(i));
                    }
                }
                None
            }

            GameMode::GunGame => {
                // First worm to complete all 5 weapons (5 kills) wins.
                for (i, w) in self.worms.iter().enumerate() {
                    if w.kills >= 5 {
                        return Some(GameResult::WormWins(i));
                    }
                }
                None
            }

            GameMode::Race { laps, .. } => {
                // First worm to complete all laps wins.
                for (i, &lap_count) in self.worm_laps.iter().enumerate() {
                    if lap_count >= *laps {
                        return Some(GameResult::WormWins(i));
                    }
                }
                None
            }

            GameMode::GameOfTag { time_limit } => {
                // Game of Tag: last worm standing when time expires wins.
                if self.cycles >= *time_limit {
                    let living: Vec<usize> = self.worms.iter().enumerate()
                        .filter(|(_, w)| w.lives > 0)
                        .map(|(i, _)| i)
                        .collect();
                    match living.len() {
                        1 => Some(GameResult::WormWins(living[0])),
                        0 => Some(GameResult::Draw),
                        _ => None,
                    }
                } else {
                    None
                }
            }

            GameMode::ScalesOfJustice => {
                // Scales of Justice: last worm standing wins.
                let living: Vec<usize> = self.worms.iter().enumerate()
                    .filter(|(_, w)| w.lives > 0)
                    .map(|(i, _)| i)
                    .collect();
                match living.len() {
                    1 => Some(GameResult::WormWins(living[0])),
                    0 => Some(GameResult::Draw),
                    _ => None,
                }
            }
        };

        // If a result was found, register it and start the countdown.
        if let Some(result) = result_opt {
            self.register_result(result);
        }

        None
    }

    /// Paint blood stain on terrain near the given position.
    ///
    /// Finds the nearest terrain pixel and stains 1-3 adjacent pixels with dark red.
    /// Only executed with 1/3 probability to avoid oversaturating terrain.
    fn paint_blood_stain(&mut self, x: i32, y: i32) {
        // Only stain 1 in 3 times to avoid overloading terrain.
        if self.rand.rand(3) != 0 { return; }

        let x = x.clamp(0, WIDTH as i32 - 1);
        let y = y.clamp(0, HEIGHT as i32 - 1);

        // Find nearest non-empty terrain pixel within a 5px radius.
        let mut found_px = None;
        for dy in -5..=5 {
            for dx in -5..=5 {
                let nx = x + dx;
                let ny = y + dy;
                if nx >= 0 && nx < WIDTH as i32 && ny >= 0 && ny < HEIGHT as i32 {
                    let pix = self.level.pixel(nx, ny);
                    // Empty pixels are 0 (air); stain if it's terrain.
                    if pix != 0 {
                        found_px = Some((nx, ny));
                        break;
                    }
                }
            }
            if found_px.is_some() { break; }
        }

        if let Some((px, py)) = found_px {
            // Paint 1-3 adjacent pixels with dark red (palette index ~45 for dark red).
            let stain_color = 45;  // Dark red palette entry
            let num_stains = (self.rand.rand(3) as usize) + 1;  // 1, 2, or 3 stains
            for _ in 0..num_stains {
                let ox = (self.rand.rand(5) as i32) - 2;
                let oy = (self.rand.rand(5) as i32) - 2;
                let sx = (px + ox).clamp(0, WIDTH as i32 - 1);
                let sy = (py + oy).clamp(0, HEIGHT as i32 - 1);
                let existing_pix = self.level.pixel(sx, sy);
                // Only stain terrain pixels (non-empty).
                if existing_pix != 0 {
                    self.level.set_pixel(sx, sy, stain_color);
                }
            }
        }
    }

    /// Paint scorch marks on terrain around an explosion position.
    ///
    /// Paints 3-5 dark grey pixels in a small radius as explosion scorch marks.
    fn paint_scorch_marks(&mut self, x: i32, y: i32) {
        let x = x.clamp(0, WIDTH as i32 - 1);
        let y = y.clamp(0, HEIGHT as i32 - 1);

        let scorch_color = 15;  // Dark grey palette entry
        let num_scorches = (self.rand.rand(3) as usize) + 3;  // 3, 4, or 5 scorches

        for _ in 0..num_scorches {
            let ox = (self.rand.rand(5) as i32) - 2;
            let oy = (self.rand.rand(5) as i32) - 2;
            let sx = (x + ox).clamp(0, WIDTH as i32 - 1);
            let sy = (y + oy).clamp(0, HEIGHT as i32 - 1);
            let existing_pix = self.level.pixel(sx, sy);
            // Only scorch terrain pixels (non-empty).
            if existing_pix != 0 {
                self.level.set_pixel(sx, sy, scorch_color);
            }
        }
    }

    /// Randomize weapon loadout for all worms if random_weapons is enabled.
    ///
    /// Uses Fisher-Yates shuffle to pick 5 random weapon indices from the TC's weapon list.
    /// Only executes if `self.random_weapons` is true and TC has at least 5 weapons.
    pub fn randomize_weapon_slots(&mut self) {
        if !self.random_weapons { return; }
        let num_weapons = self.tc.weapons.len();
        if num_weapons < 5 { return; }

        for worm in self.worms.iter_mut() {
            // Fisher-Yates shuffle: build a vector of indices and shuffle them.
            let mut indices: Vec<usize> = (0..num_weapons).collect();
            for i in (1..num_weapons).rev() {
                let j = self.rand.rand((i + 1) as u32) as usize;
                indices.swap(i, j);
            }
            // Pick the first 5 shuffled indices as the weapon loadout.
            worm.weapon_slots = [indices[0], indices[1], indices[2], indices[3], indices[4]];
        }
    }
}
