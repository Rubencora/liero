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
use crate::worm::{Worm, react};
use crate::wobject::WObject;
use liero_data::Tc;

/// Maximum simultaneous worms (4-player support).
pub const MAX_WORMS: usize = 4;

// ── Game mode types ───────────────────────────────────────────────────────────

// ── Rollback snapshot ─────────────────────────────────────────────────────────

/// Full simulation state snapshot for rollback netcode.
///
/// Contains every mutable field of `Game` except `tc` (constant) and
/// `tc_materials` (derived from `tc`).  A 16-slot ring buffer of these
/// snapshots costs ~16 × 180 KB ≈ 2.9 MB — acceptable for desktop.
#[derive(Clone)]
pub struct GameSnapshot {
    pub rand:         crate::rand::Mwc,
    pub cycles:       i32,
    pub worms:        Vec<crate::worm::Worm>,
    pub wobjects:     Vec<crate::wobject::WObject>,
    pub nobjects:     Vec<crate::nobject::NObject>,
    pub bonuses:      Vec<Option<Bonus>>,
    pub level_pixels: Vec<u8>,
    pub koth_ticks:   [i32; 4],
    pub bomb_timer:   i32,
    pub dirt_scores:  [i32; 4],
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
    /// Active pickup bonuses (weapon crates / health packs).
    /// Uses pool semantics: `None` slots are freed bonuses (do not remove/swap).
    pub bonuses:  Vec<Option<Bonus>>,
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

    // ── Material palette cache ───────────────────────────────────────────────
    /// Material flags per palette index, derived from `tc.data.constants.materials`.
    /// Stored here to avoid repeated lookups through `tc`.
    pub tc_materials: Vec<Material>,
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

        Self {
            level,
            worms,
            wobjects:     Vec::new(),
            nobjects:     Vec::new(),
            bonuses:      Vec::new(),
            rand,
            cycles:       0,
            seed,
            max_bonuses:  4,
            sound_events: Vec::new(),
            mode:         GameMode::default(),
            koth_ticks:   [0; 4],
            bomb_timer:   0,
            dirt_scores:  [0; 4],
            tc_materials,
            tc,
        }
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

        // ── 2. Process WObjects then NObjects ────────────────────────────────────
        // Mirrors C++ processFrame order: bonuses → sobjects → wobjects → nobjects → ++cycles.
        self.step_wobjects();
        self.step_nobjects();

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
                // Alive: run terrain physics.
                Self::step_worm_physics(
                    &mut self.worms[wi],
                    &self.level,
                    &self.tc_materials,
                    &self.tc.data.constants,
                );

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
                                    // Apply healing (health is in the checksum).
                                    self.worms[wi].health =
                                        (w_health + heal).min(max_health);
                                    self.sound_events.push(SoundEvent::BonusCollect);
                                    // Don't advance bi.
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
                    for rt in &mut self.worms[wi].reload_timers {
                        if *rt > 0 { *rt -= 1; }
                    }

                    let cur_weapon = self.worms[wi].current_weapon as usize;
                    let can_fire   = self.worms[wi].reload_timers.get(cur_weapon).copied().unwrap_or(0) <= 0;

                    if inp & input::FIRE != 0 && can_fire {
                        if let Some(weapon) = self.tc.weapons.get(cur_weapon) {
                            let loading_time = weapon.loading_time;
                            let angle        = self.worms[wi].aiming_angle as usize & 0x7f;
                            let worm_vel     = self.worms[wi].vel;
                            let parts        = weapon.parts.max(1);
                            let distribution = weapon.distribution;

                            // Set reload timer.
                            let slot = cur_weapon.min(4);
                            self.worms[wi].reload_timers[slot] = loading_time;

                            // Recoil (C++: worm.vel -= cossinTable[angle] * recoil / 100).
                            let recoil = weapon.recoil;
                            if recoil != 0 {
                                let (cx, cy) = COSSIN_TABLE[angle];
                                self.worms[wi].vel.x.0 -= ((cx as i64 * recoil as i64) / 100) as i32;
                                self.worms[wi].vel.y.0 -= ((cy as i64 * recoil as i64) / 100) as i32;
                            }

                            // Emit weapon-fire sound (once per shot burst, not per part).
                            self.sound_events.push(SoundEvent::WeaponFire { weapon_idx: cur_weapon });

                            // Fire each part (spread via distribution).
                            for _ in 0..parts {
                                let fire_angle = if distribution > 0 {
                                    let spread = self.rand.rand((distribution * 2) as u32) as i32 - distribution;
                                    ((angle as i32 + spread).rem_euclid(128)) as usize
                                } else {
                                    angle
                                };
                                self.create_wobject(cur_weapon, wi, fire_angle, worm_vel);
                            }
                        }
                    }
                }

                // ── Rendering state (not part of checksum) ───────────────────────────
                {
                    use crate::worm::input;
                    let w = &mut self.worms[wi];
                    if inp & input::LEFT  != 0 { w.direction = 0; }
                    if inp & input::RIGHT != 0 { w.direction = 1; }

                    let on_ground = w.reacts[react::UP] > 0;
                    let moving    = inp & (input::LEFT | input::RIGHT) != 0;
                    w.animate     = on_ground && moving;

                    let aiming = w.aiming_angle;
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
            } else {
                // Dead: respawn state machine mirrors C++ Worm::process() dead branch.
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
            // All other modes: respawn only while lives remain.
            _ => self.worms[wi].lives > 0,
        }
    }

    /// Per-frame game-mode side-effects: KotH zone ticks, BombTag fuse countdown.
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
                    // blood_pct = Settings::blood = 100 (headless default).
                    let blood_amount = (100 * power_sum / 100) as usize;

                    if self.worms[wi].health > 0 {
                        // Extract worm state before mutable borrow of self.
                        let worm_pos = self.worms[wi].pos;
                        let worm_vel = self.worms[wi].vel;
                        for _ in 0..blood_amount {
                            let angle = self.rand.rand(128) as usize;
                            // C++: vel_in = w.vel / 3 (raw fixed-point divide by 3)
                            let vel_in = FixedVec { x: Fixed(worm_vel.x.0 / 3), y: Fixed(worm_vel.y.0 / 3) };
                            self.create_nobject2(blood_nobj_idx, angle, vel_in, worm_pos);
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

        // 4a. startFrame > 0: consume rand(numFrames+1) for curFrame (visual, not tracked).
        if nobj_type.start_frame > 0 {
            self.rand.rand(nobj_type.num_frames as u32 + 1);
        }

        // 4b. timeToExplo / timeToExploV.
        let mut time_left = nobj_type.time_to_explo;
        if nobj_type.time_to_explo_v != 0 {
            time_left -= self.rand.rand(nobj_type.time_to_explo_v as u32) as i32;
        }

        // 5. Initial position advance: obj.pos = pos + vel (C++: obj.pos = pos; obj.pos += obj.vel).
        let final_pos = pos + vel;

        self.nobjects.push(NObject::new(nobj_idx, final_pos, vel, time_left));
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
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
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

            // ── 7. hitDamage — skipped (headless sim; blood/particle have hitDamage=0) ─

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

        // TDM: no friendly fire.
        if matches!(self.mode, GameMode::TeamDeathmatch) {
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
        actual = actual.max(1);

        self.worms[worm_idx].health -= actual;

        if self.worms[worm_idx].health <= 0 {
            self.worms[worm_idx].health       = 0;
            self.worms[worm_idx].alive        = false;
            self.worms[worm_idx].killed_timer = 150;
            self.worms[worm_idx].lives        -= 1;

            // Kill credit.
            if by_idx >= 0 {
                let a = by_idx as usize;
                if a != worm_idx && a < self.worms.len() {
                    self.worms[a].kills += 1;
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
                    self.worms[new_jug].health = 300; // new juggernaut resets to 3× HP
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
    fn step_wobjects(&mut self) {
        let mut i = 0;
        while i < self.wobjects.len() {
            let weapon = match self.tc.weapons.get(self.wobjects[i].weapon_idx) {
                Some(w) => w.clone(),
                None    => { i += 1; continue; }
            };

            // ── 1. pos += vel ─────────────────────────────────────────────────
            let vel = self.wobjects[i].vel;
            self.wobjects[i].pos += vel;

            let pos    = self.wobjects[i].pos;
            let ipos_x = pos.x.to_int();
            let ipos_y = pos.y.to_int();

            // ── 2. Terrain collision ───────────────────────────────────────────
            let in_terrain = !self.level.inside(ipos_x, ipos_y)
                || self.level.material(ipos_x, ipos_y, &self.tc_materials).dirt_rock();

            let mut do_explode = false;
            let mut do_remove  = false;

            if in_terrain && !weapon.pierce_dirt {
                if weapon.expl_ground {
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
            if !do_explode && !do_remove && weapon.boomerang_return_force > 0 {
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
            if !do_explode && !do_remove && (weapon.hit_damage > 0 || weapon.worm_collide) {
                let owner_idx = self.wobjects[i].owner_idx;
                let n_worms   = self.worms.len();
                'worm_loop: for wi in 0..n_worms {
                    if !self.worms[wi].alive { continue; }
                    // Skip worm that fired (detect_distance == 0 prevents self-hit early on).
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
                        // Damage.
                        self.do_damage(wi, weapon.hit_damage, owner_idx as i32);

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
                            let wvel = self.worms[wi].vel;
                            for _ in 0..weapon.blood_on_hit {
                                let angle  = self.rand.rand(128) as usize;
                                let vel_in = FixedVec { x: Fixed(wvel.x.0 / 3), y: Fixed(wvel.y.0 / 3) };
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
                            for _ in 0..weapon.splinter_amount {
                                let angle     = self.rand.rand(128) as usize;
                                let color_sub = self.rand.rand(2);
                                let _color    = weapon.splinter_colour - color_sub as i32;
                                self.create_nobject2(sidx, angle, FixedVec::ZERO, spos);
                            }
                        }
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
            let s = speed.max(100);
            FixedVec {
                x: Fixed(base_vel.x.0 + (extra_vel.x.0 as i64 * 100 / s) as i32),
                y: Fixed(base_vel.y.0 + (extra_vel.y.0 as i64 * 100 / s) as i32),
            }
        } else {
            base_vel
        };

        // Firing position: detect_distance + 5 pixels ahead of worm, 1 pixel up.
        let wpos = self.worms[owner_idx].pos;
        let fire_dist = weapon.detect_distance + 5;
        let fire_pos = FixedVec {
            x: Fixed(wpos.x.0 + ((cx as i64 * fire_dist as i64) / 65536) as i32),
            y: Fixed(wpos.y.0 + ((cy as i64 * fire_dist as i64) / 65536) as i32 - (1 << 16)),
        };

        // time_left = timeToExplo - rand(timeToExploV).
        let time_left = if weapon.time_to_explo > 0 {
            weapon.time_to_explo - self.rand.rand(weapon.time_to_explo_v.max(1) as u32) as i32
        } else {
            0
        };

        let wobj = WObject::new(weapon_idx, owner_idx, fire_pos, proj_vel, time_left);
        self.wobjects.push(wobj);
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
                // Determine frame (0 = weapon, 1 = health).
                let frame = if only_health {
                    1
                } else if only_weapon {
                    0
                } else {
                    self.rand.rand(2) as i32
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

        // Gravity (when not standing on ground).
        if worm.reacts[react::UP] == 0 {
            worm.vel.y += Fixed(consts.worm_gravity);
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
            rand:         self.rand.clone(),
            cycles:       self.cycles,
            worms:        self.worms.clone(),
            wobjects:     self.wobjects.clone(),
            nobjects:     self.nobjects.clone(),
            bonuses:      self.bonuses.clone(),
            level_pixels: self.level.pixels().to_vec(),
            koth_ticks:   self.koth_ticks,
            bomb_timer:   self.bomb_timer,
            dirt_scores:  self.dirt_scores,
        }
    }

    /// Overwrite mutable state from a previously saved `GameSnapshot`.
    ///
    /// The `tc` and `tc_materials` fields are left unchanged (they are constant).
    pub fn restore_snapshot(&mut self, snap: &GameSnapshot) {
        self.rand        = snap.rand.clone();
        self.cycles      = snap.cycles;
        self.worms       = snap.worms.clone();
        self.wobjects    = snap.wobjects.clone();
        self.nobjects    = snap.nobjects.clone();
        self.bonuses     = snap.bonuses.clone();
        self.koth_ticks  = snap.koth_ticks;
        self.bomb_timer  = snap.bomb_timer;
        self.dirt_scores = snap.dirt_scores;
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
                    w.health        = 300;
                }
            }
            GameMode::BombTag { fuse } => {
                // Worm 0 starts with the bomb; fuse starts ticking immediately.
                self.bomb_timer = *fuse;
                if let Some(w) = self.worms.first_mut() {
                    w.has_bomb = true;
                }
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
    pub fn game_over(&self) -> Option<GameResult> {
        // Need at least two worms for a meaningful game.
        if self.worms.len() < 2 { return None; }

        match &self.mode {
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
        }
    }
}
