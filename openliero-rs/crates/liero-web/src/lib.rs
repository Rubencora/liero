//! WebAssembly entry point for OpenLiero — Canvas 2D rendering path (no wgpu).
//!
//! Embeds the TC directory at compile time via `include_dir!`, initialises
//! a SoftRenderer (CPU-based), and drives the same `liero-sim` game logic
//! as the native desktop build.
//!
//! Input is handled entirely from JavaScript (keydown/keyup on window) via
//! `set_inputs()` and `fire_ui_action()`. This bypasses winit's WASM keyboard
//! handling, which is unreliable across browser/OS combinations.
//!
//! Online multiplayer is handled by `net::WsSession` — a rollback netcode
//! session backed by a WebSocket relay connection.
//!
//! Build with:
//!   wasm-pack build openliero-rs/crates/liero-web --target web --out-dir ../../../web/play/pkg

mod net;

use std::cell::RefCell;
use anyhow::anyhow;
use include_dir::{include_dir, Dir};
use liero_data::Tc;
use liero_render::SoftRenderer;
use liero_sim::game::{Game, GameMode, GameResult, SoundEvent};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use net::WsSession;
use serde_json;

// Embed TC at compile time.
static TC_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../../TC/openliero");

fn load_embedded_tc() -> anyhow::Result<Tc> {
    Tc::load_with(|rel_path| {
        TC_DIR.get_file(rel_path)
            .map(|f| f.contents().to_vec())
            .ok_or_else(|| anyhow!("embedded TC file not found: {rel_path}"))
    })
}

// ── JS-driven state ───────────────────────────────────────────────────────────

thread_local! {
    static JS_INPUTS:        RefCell<[u32; 4]>          = RefCell::new([0; 4]);
    static JS_UI_EVENTS:     RefCell<Vec<UiAction>>      = RefCell::new(Vec::new());
    static JS_PLAYER_COUNT:  RefCell<usize>               = RefCell::new(2);
    static WS_SESSION:       RefCell<Option<WsSession>>  = RefCell::new(None);
    static JS_GAME_MODE:     RefCell<GameMode>            = RefCell::new(GameMode::LastManStanding);
    static JS_WEAPON_SLOTS:  RefCell<[[usize; 5]; 4]>    = RefCell::new([[0,1,2,3,4]; 4]);
    static JS_GAME_SEED:     RefCell<u32>                = RefCell::new(42);
    static JS_WORM_HEALTH:        RefCell<i32>  = RefCell::new(100);
    static JS_WORM_LIVES:         RefCell<i32>  = RefCell::new(3);
    static JS_BLOOD_PCT:          RefCell<i32>  = RefCell::new(100);
    static JS_LOADING_TIME_FACTOR: RefCell<i32> = RefCell::new(100);
    static JS_MAX_BONUSES:        RefCell<i32>  = RefCell::new(4);
    static JS_FRIENDLY_FIRE:      RefCell<bool> = RefCell::new(true);
    static JS_RANDOM_WEAPONS:     RefCell<bool> = RefCell::new(false);
    static JS_NAMES_ON_BONUSES:   RefCell<bool> = RefCell::new(true);
    static JS_ALLOW_VIEWING_SPAWN: RefCell<bool> = RefCell::new(true);
    /// Pre-allocated RGBA output buffer (320×200×4 = 256 000 bytes).
    static RGBA_BUF:         RefCell<Vec<u8>>             = RefCell::new(vec![0u8; 320 * 200 * 4]);
    /// Main app state — None before init.
    static APP:              RefCell<Option<AppState>>    = RefCell::new(None);
}

#[derive(Clone)]
enum UiAction {
    Start,
    Menu,
    OnlineLobby,
    OnlineStart { local_player: usize },
}

#[derive(Debug)]
enum WebGameState {
    Menu,
    OnlineLobby,
    Playing,
    OnlinePlaying,
    GameOver { msg: String, scores: Vec<(i32, i32)> },
}

struct AppState {
    renderer: SoftRenderer,
    game_state: WebGameState,
    game: Option<Game>,
    tc: Tc,
    online_local_player: usize,
    tick_accum_ms: f64,
    first_frame_dispatched: bool,
}

// ── JS-callable API ───────────────────────────────────────────────────────────

#[wasm_bindgen]
pub fn set_inputs(p0: u32, p1: u32, p2: u32, p3: u32) {
    JS_INPUTS.with(|i| *i.borrow_mut() = [p0, p1, p2, p3]);
}

#[wasm_bindgen]
pub fn fire_ui_action(action: &str) {
    let ev = match action {
        "start" => UiAction::Start,
        "menu" => UiAction::Menu,
        "online-lobby" => UiAction::OnlineLobby,
        _ => return,
    };
    JS_UI_EVENTS.with(|v| v.borrow_mut().push(ev));
}

#[wasm_bindgen]
pub fn set_player_count(n: u32) {
    let n = (n as usize).clamp(2, 4);
    JS_PLAYER_COUNT.with(|c| *c.borrow_mut() = n);
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            app.renderer.set_player_count(n);
        }
    });
}

#[wasm_bindgen]
pub fn get_weapon_names() -> String {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let names: Vec<String> = app.tc.weapons.iter().map(|w| w.name.clone()).collect();
            serde_json::to_string(&names).unwrap_or_else(|_| "[]".into())
        } else {
            "[]".into()
        }
    })
}

/// Get weapon data as JSON for the TC editor UI.
/// Returns array of {idx, name, hit_damage, blow_away, speed, add_speed, reload_time, ammo, parts, fire_cone}
#[wasm_bindgen]
pub fn get_weapons_json() -> String {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            let weapons: Vec<serde_json::Value> = app.tc.weapons.iter().enumerate().map(|(i, w)| {
                serde_json::json!({
                    "idx": i,
                    "name": w.name,
                    "hit_damage": w.hit_damage,
                    "blow_away": w.blow_away,
                    "speed": w.speed,
                    "add_speed": w.add_speed,
                    "reload_time": w.loading_time,
                    "ammo": w.ammo,
                    "parts": w.parts,
                    "fire_cone": w.fire_cone,
                })
            }).collect();
            serde_json::to_string(&weapons).unwrap_or_else(|_| "[]".into())
        } else {
            "[]".into()
        }
    })
}

/// Modify a single weapon field at runtime.
/// field: "hit_damage", "blow_away", "speed", "add_speed", "reload_time", "ammo", "parts", "fire_cone"
/// value: the new integer value
#[wasm_bindgen]
pub fn set_weapon_field(weapon_idx: u32, field: &str, value: i32) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut() {
            let idx = weapon_idx as usize;
            if idx >= app.tc.weapons.len() { return; }
            let w = &mut app.tc.weapons[idx];
            match field {
                "hit_damage"  => w.hit_damage = value,
                "blow_away"   => w.blow_away = value,
                "speed"       => w.speed = value,
                "add_speed"   => w.add_speed = value,
                "reload_time" => w.loading_time = value,
                "ammo"        => w.ammo = value,
                "parts"       => w.parts = value,
                "fire_cone"   => w.fire_cone = value,
                _ => {}
            }
        }
    });
}

/// Reset all weapon modifications to original loaded TC values.
/// Note: Currently not implemented — would require storing original TC state.
#[wasm_bindgen]
pub fn reset_weapon_mods() {
    // TODO: This would require storing the original TC at startup and restoring from it.
    // For now, this is a placeholder for future implementation.
}

/// Set bot difficulty for a worm slot (0=Easy, 1=Medium, 2=Hard).
#[wasm_bindgen]
pub fn set_worm_bot_difficulty(player: u32, difficulty: i32) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut() {
            if let Some(game) = app.game.as_mut() {
                let pi = player as usize;
                if pi < game.worms.len() {
                    game.worms[pi].bot_difficulty = difficulty;
                }
            }
        }
    });
}

/// Get embed mode configuration as JSON.
#[wasm_bindgen]
pub fn get_embed_config() -> String {
    r#"{"embed": true}"#.to_string()
}

#[wasm_bindgen]
pub fn set_worm_weapon_slots(player: u32, w0: u32, w1: u32, w2: u32, w3: u32, w4: u32) {
    let pi = (player as usize).min(3);
    JS_WEAPON_SLOTS.with(|ws| {
        ws.borrow_mut()[pi] = [w0 as usize, w1 as usize, w2 as usize, w3 as usize, w4 as usize]
    });
}

#[wasm_bindgen]
pub fn set_game_mode(mode: &str, param: i32) {
    let m = match mode {
        "kill-race"          => GameMode::KillRace    { frag_limit: param.max(1) },
        "team-deathmatch"    => GameMode::TeamDeathmatch,
        "king-of-hill"       => GameMode::KingOfHill  { time_limit: param.max(30) * 60 },
        "bomb-tag"           => GameMode::BombTag      { fuse: param.max(5) * 60 },
        "zombie-mode"        => GameMode::ZombieMode,
        "juggernaut"         => GameMode::Juggernaut,
        "dirt-war"           => GameMode::DirtWar      { time_limit: param.max(30) * 60 },
        "gun-game"           => GameMode::GunGame,
        "race"               => GameMode::Race {
            checkpoints: vec![
                (80, 40), (240, 40), (240, 120), (80, 120), (160, 80)
            ],
            laps: param.max(1),
        },
        "game-of-tag"        => GameMode::GameOfTag { time_limit: param.max(30) * 60 },
        "scales-of-justice"  => GameMode::ScalesOfJustice,
        _                    => GameMode::LastManStanding,
    };
    JS_GAME_MODE.with(|c| *c.borrow_mut() = m);
}

#[wasm_bindgen]
pub fn set_game_seed(seed: u32) {
    JS_GAME_SEED.with(|s| *s.borrow_mut() = seed);
}

/// Set per-worm starting health (default 100).
#[wasm_bindgen]
pub fn set_worm_health(health: i32) {
    JS_WORM_HEALTH.with(|h| *h.borrow_mut() = health.max(1).min(500));
}

/// Set per-worm starting lives (default 3).
#[wasm_bindgen]
pub fn set_worm_lives(lives: i32) {
    JS_WORM_LIVES.with(|l| *l.borrow_mut() = lives.max(1).min(99));
}

/// Set blood particle percentage (0-100, default 100).
#[wasm_bindgen]
pub fn set_blood_pct(pct: i32) {
    JS_BLOOD_PCT.with(|b| *b.borrow_mut() = pct.clamp(0, 100));
}

/// Set weapon loading time factor (50=half speed, 100=normal, 200=double, default 100).
#[wasm_bindgen]
pub fn set_loading_time_factor(factor: i32) {
    JS_LOADING_TIME_FACTOR.with(|f| *f.borrow_mut() = factor.clamp(10, 500));
}

/// Set maximum concurrent bonuses on the level (default 4).
#[wasm_bindgen]
pub fn set_max_bonuses(n: i32) {
    JS_MAX_BONUSES.with(|b| *b.borrow_mut() = n.clamp(0, 20));
}

/// Set friendly fire on/off (default true).
#[wasm_bindgen]
pub fn set_friendly_fire(enabled: bool) {
    JS_FRIENDLY_FIRE.with(|f| *f.borrow_mut() = enabled);
}

/// Set random weapon loadout each round (default false).
#[wasm_bindgen]
pub fn set_random_weapons(enabled: bool) {
    JS_RANDOM_WEAPONS.with(|r| *r.borrow_mut() = enabled);
}

/// Set whether bonus names should be displayed (default true).
#[wasm_bindgen]
pub fn set_names_on_bonuses(enabled: bool) {
    JS_NAMES_ON_BONUSES.with(|n| *n.borrow_mut() = enabled);
}

/// Check if names should be shown on bonuses (read by renderer).
#[wasm_bindgen]
pub fn get_names_on_bonuses() -> bool {
    JS_NAMES_ON_BONUSES.with(|n| *n.borrow())
}

/// Set whether viewing spawn point is allowed (default true).
#[wasm_bindgen]
pub fn set_allow_viewing_spawn_point(enabled: bool) {
    JS_ALLOW_VIEWING_SPAWN.with(|v| *v.borrow_mut() = enabled);
}

#[wasm_bindgen]
pub fn set_worm_bot(player: u32, is_bot: bool) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut() {
            if let Some(game) = app.game.as_mut() {
                let pi = player as usize;
                if pi < game.worms.len() {
                    game.worms[pi].is_bot = is_bot;
                }
            }
        }
    });
}

/// Get race mode progress: checkpoint and lap counts per worm.
/// Returns JSON array with { worm, checkpoint, laps } for each worm.
#[wasm_bindgen]
pub fn get_race_progress() -> String {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            if let Some(game) = app.game.as_ref() {
                let progress: Vec<serde_json::Value> = (0..game.worms.len()).map(|i| {
                    serde_json::json!({
                        "worm": i,
                        "checkpoint": game.worm_checkpoint[i],
                        "laps": game.worm_laps[i]
                    })
                }).collect();
                return serde_json::to_string(&progress).unwrap_or_else(|_| "[]".into());
            }
        }
        "[]".into()
    })
}

/// Get TC hack flags as JSON object.
#[wasm_bindgen]
pub fn get_tc_hacks() -> String {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            let h = &app.tc.data.hacks;
            serde_json::json!({
                "fall_damage": h.fall_damage,
                "bonus_reload_only": h.bonus_reload_only,
                "bonus_spawn_rect": h.bonus_spawn_rect,
                "bonus_only_health": h.bonus_only_health,
                "bonus_only_weapon": h.bonus_only_weapon,
                "bonus_disable": h.bonus_disable,
                "worm_float": h.worm_float,
                "rem_exp": h.rem_exp,
                "signed_recoil": h.signed_recoil,
                "air_jump": h.air_jump,
                "multi_jump": h.multi_jump,
            }).to_string()
        } else {
            "{}".to_string()
        }
    })
}

/// Get Game of Tag status: which worm is "it" and their timer.
#[wasm_bindgen]
pub fn get_tag_status() -> String {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            if let Some(game) = app.game.as_ref() {
                let tag_worm = game.tag_worm.unwrap_or(usize::MAX);
                let timer = if let Some(idx) = game.tag_worm {
                    game.worms.get(idx).map(|w| w.timer).unwrap_or(0)
                } else { 0 };
                return serde_json::json!({
                    "tag_worm": tag_worm,
                    "timer": timer
                }).to_string();
            }
        }
        r#"{"tag_worm":255,"timer":0}"#.to_string()
    })
}

/// Returns a pointer to the pre-allocated RGBA output buffer in WASM memory.
#[wasm_bindgen]
pub fn get_rgba_ptr() -> u32 {
    RGBA_BUF.with(|b| b.borrow().as_ptr() as u32)
}

/// Returns the byte length of the RGBA output buffer (always 320*200*4).
#[wasm_bindgen]
pub fn get_rgba_len() -> u32 {
    (320 * 200 * 4) as u32
}

/// Expose the WebAssembly memory object so JS can read RGBA data without copying.
#[wasm_bindgen]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// Get all level pixels as a byte array (320×200 palette indices).
/// Used by the level editor to read the current level.
#[wasm_bindgen]
pub fn get_level_pixels() -> Vec<u8> {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            if let Some(game) = app.game.as_ref() {
                return game.level.pixels().to_vec();
            }
        }
        vec![0u8; 320 * 200]
    })
}

/// Set all level pixels from a byte array (320×200 palette indices).
/// Used by the level editor to apply changes.
#[wasm_bindgen]
pub fn set_level_pixels(data: &[u8]) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut() {
            if let Some(game) = app.game.as_mut() {
                if data.len() == 320 * 200 {
                    game.level.restore_pixels(data);
                }
            }
        }
    });
}

/// Advance the simulation one frame, compose the scene, apply palette to RGBA buffer.
/// Returns `true` the FIRST time this runs during a Playing/OnlinePlaying session
/// (used by JS to know when to hide the loading overlay).
#[wasm_bindgen]
pub fn step_frame() -> bool {
    let mut first_frame = false;

    APP.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let app = match borrow.as_mut() {
            Some(a) => a,
            None => return,
        };

        // ── 1. Process queued UI actions ─────────────────────────────────────
        let ui_events: Vec<UiAction> = JS_UI_EVENTS.with(|v| v.borrow_mut().drain(..).collect());
        for ev in ui_events {
            match ev {
                UiAction::Start
                    if matches!(
                        app.game_state,
                        WebGameState::Menu | WebGameState::GameOver { .. }
                    ) =>
                {
                    let tc = app.tc.clone();
                    let player_count = JS_PLAYER_COUNT.with(|c| *c.borrow());
                    let seed = JS_GAME_SEED.with(|s| *s.borrow());
                    let mut game = Game::new(tc, player_count, seed);
                    game.set_mode(JS_GAME_MODE.with(|m| m.borrow().clone()));
                    let wslots = JS_WEAPON_SLOTS.with(|ws| *ws.borrow());
                    for (wi, worm) in game.worms.iter_mut().enumerate() {
                        if wi < 4 {
                            worm.weapon_slots = wslots[wi];
                        }
                    }

                    // Apply match settings from JavaScript
                    let health = JS_WORM_HEALTH.with(|h| *h.borrow());
                    let lives = JS_WORM_LIVES.with(|l| *l.borrow());
                    let blood = JS_BLOOD_PCT.with(|b| *b.borrow());
                    let loading = JS_LOADING_TIME_FACTOR.with(|f| *f.borrow());
                    let bonuses = JS_MAX_BONUSES.with(|b| *b.borrow());
                    let ff = JS_FRIENDLY_FIRE.with(|f| *f.borrow());
                    let rw = JS_RANDOM_WEAPONS.with(|r| *r.borrow());

                    // Apply to worms
                    for worm in game.worms.iter_mut() {
                        worm.health = health;
                        worm.lives = lives;
                    }

                    // Apply to game fields (only set if they exist in the Game struct)
                    game.blood_pct = blood;
                    game.loading_time_factor = loading;
                    game.max_bonuses = bonuses;
                    game.friendly_fire = ff;
                    game.random_weapons = rw;

                    app.game = Some(game);
                    app.game_state = WebGameState::Playing;
                    app.first_frame_dispatched = false;
                    JS_INPUTS.with(|i| *i.borrow_mut() = [0; 4]);
                }
                UiAction::Menu => {
                    WS_SESSION.with(|ws| *ws.borrow_mut() = None);
                    app.game = None;
                    app.game_state = WebGameState::Menu;
                    app.first_frame_dispatched = false;
                    JS_INPUTS.with(|i| *i.borrow_mut() = [0; 4]);
                }
                UiAction::OnlineLobby => {
                    app.game_state = WebGameState::OnlineLobby;
                }
                UiAction::OnlineStart { local_player } => {
                    let tc = app.tc.clone();
                    let mut game = Game::new(tc, 2, 42);
                    game.set_mode(GameMode::LastManStanding);
                    app.game = Some(game);
                    app.online_local_player = local_player;
                    app.game_state = WebGameState::OnlinePlaying;
                    app.first_frame_dispatched = false;
                    JS_INPUTS.with(|i| *i.borrow_mut() = [0; 4]);
                }
                _ => {}
            }
        }

        // ── 2. Fixed-timestep simulation (60 Hz) ─────────────────────────────
        const STEP_MS: f64 = 1000.0 / 60.0;
        const MAX_STEPS: u32 = 4;

        let now_ms: f64 = web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0);

        let delta = if app.tick_accum_ms < 0.0 {
            STEP_MS
        } else {
            (now_ms - app.tick_accum_ms).min(STEP_MS * MAX_STEPS as f64)
        };
        app.tick_accum_ms = now_ms;

        thread_local! { static FRAC: RefCell<f64> = RefCell::new(0.0); }
        let sim_steps = FRAC.with(|frac| {
            *frac.borrow_mut() += delta;
            let mut n = 0u32;
            while *frac.borrow() >= STEP_MS && n < MAX_STEPS {
                *frac.borrow_mut() -= STEP_MS;
                n += 1;
            }
            n
        });

        let mut game_over: Option<GameResult> = None;
        for _ in 0..sim_steps {
            let r = match &app.game_state {
                WebGameState::Playing => {
                    let inputs = JS_INPUTS.with(|i| *i.borrow());
                    app.game.as_mut().and_then(|g| {
                        g.step(&inputs);
                        g.game_over()
                    })
                }
                WebGameState::OnlinePlaying => {
                    let local_raw = JS_INPUTS.with(|i| i.borrow()[0]);
                    WS_SESSION.with(|ws| {
                        ws.borrow_mut().as_mut().and_then(|session| {
                            session.step(app.game.as_mut()?, local_raw);
                            app.game.as_mut()?.game_over()
                        })
                    })
                }
                _ => None,
            };
            if r.is_some() {
                game_over = r;
                break;
            }
        }

        if let Some(result) = game_over {
            WS_SESSION.with(|ws| *ws.borrow_mut() = None);
            let scores = app
                .game
                .as_ref()
                .map(|g| g.worms.iter().map(|w| (w.kills, w.deaths)).collect::<Vec<_>>())
                .unwrap_or_default();
            app.game_state = WebGameState::GameOver {
                msg: format_result(result),
                scores,
            };
        }

        // ── 3. Compose frame ─────────────────────────────────────────────────
        let playing = matches!(
            app.game_state,
            WebGameState::Playing | WebGameState::OnlinePlaying
        );

        match &app.game_state {
            WebGameState::Menu => {
                let player_count = JS_PLAYER_COUNT.with(|c| *c.borrow());
                let subtitle = format!("{} PLAYERS", player_count);
                app.renderer.render_menu(
                    "OPENLIERO",
                    &["PLAY LOCAL", "PLAY ONLINE"],
                    0,
                    &["PRESS ENTER OR SPACE TO START", &subtitle],
                );
            }
            WebGameState::OnlineLobby => {
                app.renderer
                    .render_menu("ONLINE LOBBY", &[], 0, &["USE THE BROWSER UI"]);
            }
            WebGameState::Playing | WebGameState::OnlinePlaying => {
                if let Some(g) = app.game.as_ref() {
                    app.renderer.render(g);
                }
            }
            WebGameState::GameOver { msg, scores } => {
                let msg = msg.clone();
                let scores = scores.clone();
                app.renderer.render_game_over(&msg, &scores);
            }
        }

        // Apply palette → RGBA buffer
        RGBA_BUF.with(|buf| {
            app.renderer.write_rgba(&mut buf.borrow_mut());
        });

        // Signal first Playing frame
        if playing && !app.first_frame_dispatched {
            app.first_frame_dispatched = true;
            first_frame = true;
        }
    });

    first_frame
}

/// Drain sound events produced by the last `step_frame()` call.
///
/// Returns a JSON array of objects: `[{"type":"fire","weapon":3}, {"type":"hurt"}, ...]`
/// JS calls this after each step and triggers Web Audio accordingly.
#[wasm_bindgen]
pub fn drain_sound_events() -> String {
    let mut events: Vec<String> = Vec::new();
    APP.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let app = match borrow.as_mut() {
            Some(a) => a,
            None => return,
        };
        if let Some(game) = app.game.as_mut() {
            for ev in game.sound_events.drain(..) {
                let s = match ev {
                    SoundEvent::WeaponFire { weapon_idx } =>
                        format!(r#"{{"type":"fire","weapon":{}}}"#, weapon_idx),
                    SoundEvent::Explosion { sobj_idx } =>
                        format!(r#"{{"type":"explosion","sobj":{}}}"#, sobj_idx),
                    SoundEvent::Hurt { .. } =>
                        r#"{"type":"hurt"}"#.to_string(),
                    SoundEvent::Death { .. } =>
                        r#"{"type":"death"}"#.to_string(),
                    SoundEvent::BonusCollect =>
                        r#"{"type":"bonus"}"#.to_string(),
                    SoundEvent::WormBounce { worm_idx } =>
                        format!(r#"{{"type":"bounce","worm":{}}}"#, worm_idx),
                    SoundEvent::WormRespawn { worm_idx } =>
                        format!(r#"{{"type":"respawn","worm":{}}}"#, worm_idx),
                    SoundEvent::WeaponReloaded { worm_idx } =>
                        format!(r#"{{"type":"reloaded","worm":{}}}"#, worm_idx),
                    SoundEvent::RopeThrown { worm_idx } =>
                        format!(r#"{{"type":"rope","worm":{}}}"#, worm_idx),
                };
                events.push(s);
            }
        }
    });
    format!("[{}]", events.join(","))
}

fn format_result(r: GameResult) -> String {
    match r {
        GameResult::WormWins(i) => format!("PLAYER {} WINS!", i + 1),
        GameResult::TeamWins(t) => format!("TEAM {} WINS!", t + 1),
        GameResult::Draw => "DRAW!".to_string(),
    }
}

/// Returns game result as JSON string if game is over, empty string otherwise.
/// Format: {"winner": 0, "kills": [0,1,2,3], "deaths": [0,1,2,3]}
/// winner: -1 for draw, or player index (0-3)
#[wasm_bindgen]
pub fn get_game_result() -> String {
    APP.with(|cell| {
        let borrow = cell.borrow();
        let app = match borrow.as_ref() {
            Some(a) => a,
            None => return "".to_string(),
        };
        match &app.game_state {
            WebGameState::GameOver { msg: _, scores } => {
                let mut kills = Vec::new();
                let mut deaths = Vec::new();
                for (k, d) in scores {
                    kills.push(k);
                    deaths.push(d);
                }
                // Determine winner: find player with most kills (or -1 for draw)
                let winner = if kills.is_empty() {
                    -1
                } else {
                    let max_kills = kills.iter().map(|k| **k).max().unwrap_or(0);
                    let winners: Vec<usize> = kills
                        .iter()
                        .enumerate()
                        .filter(|(_, k)| ***k == max_kills)
                        .map(|(i, _)| i)
                        .collect();
                    if winners.len() == 1 {
                        winners[0] as i32
                    } else {
                        -1 // draw
                    }
                };
                let kills_str = kills.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(",");
                let deaths_str = deaths.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",");
                format!(
                    r#"{{"winner":{},"kills":[{}],"deaths":[{}]}}"#,
                    winner, kills_str, deaths_str
                )
            }
            _ => "".to_string(),
        }
    })
}

// ── Audio file access ─────────────────────────────────────────────────────────

/// Get a list of available sound file names as JSON array.
#[wasm_bindgen]
pub fn get_sound_names() -> String {
    let mut names: Vec<String> = Vec::new();
    if let Some(sounds_dir) = TC_DIR.get_dir("sounds") {
        for file in sounds_dir.files() {
            if let Some(name_str) = file.path().file_name().and_then(|n| n.to_str()) {
                if name_str.ends_with(".wav") {
                    names.push(name_str.to_string());
                }
            }
        }
    }
    serde_json::to_string(&names).unwrap_or_else(|_| "[]".into())
}

/// Get the raw bytes of a sound file by name.
/// Returns empty array if not found.
#[wasm_bindgen]
pub fn get_sound_bytes(name: &str) -> Vec<u8> {
    let path = format!("sounds/{}", name);
    TC_DIR.get_file(&path)
        .map(|f| f.contents().to_vec())
        .unwrap_or_default()
}

/// Get the sound index to file name mapping for each weapon.
/// Returns JSON: [{weapon_idx, weapon_name, launch_sound, explo_sound, loop_sound}, ...]
#[wasm_bindgen]
pub fn get_sound_mappings() -> String {
    APP.with(|cell| {
        if let Some(app) = cell.borrow().as_ref() {
            let mappings: Vec<serde_json::Value> = app.tc.weapons.iter().enumerate().map(|(i, w)| {
                serde_json::json!({
                    "weapon_idx": i,
                    "weapon_name": w.name,
                    "launch_sound": w.launch_sound,
                    "explo_sound": w.explo_sound,
                    "loop_sound": w.loop_sound,
                })
            }).collect();
            serde_json::to_string(&mappings).unwrap_or_else(|_| "[]".into())
        } else {
            "[]".into()
        }
    })
}

fn dispatch_event(name: &str, detail: Option<&str>) -> anyhow::Result<()> {
    let win = web_sys::window().ok_or_else(|| anyhow!("no window"))?;
    if let Some(d) = detail {
        let init = web_sys::CustomEventInit::new();
        init.set_detail(&JsValue::from_str(d));
        let ev = web_sys::CustomEvent::new_with_event_init_dict(name, &init)
            .map_err(|e| anyhow!("CustomEvent: {:?}", e))?;
        win.dispatch_event(&ev).ok();
    } else {
        let ev = web_sys::Event::new(name).map_err(|e| anyhow!("Event: {:?}", e))?;
        win.dispatch_event(&ev).ok();
    }
    Ok(())
}

// ── WASM entry point ──────────────────────────────────────────────────────────

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&"[liero] loading embedded TC…".into());

    let tc = match load_embedded_tc() {
        Ok(tc) => {
            web_sys::console::log_1(&"[liero] TC loaded".into());
            tc
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[liero] TC load failed: {e}").into());
            return;
        }
    };

    let player_count = JS_PLAYER_COUNT.with(|c| *c.borrow());
    let mut renderer = SoftRenderer::new(&tc);
    renderer.set_player_count(player_count);

    APP.with(|cell| {
        *cell.borrow_mut() = Some(AppState {
            renderer,
            game_state: WebGameState::Menu,
            game: None,
            tc,
            online_local_player: 0,
            tick_accum_ms: -1.0,
            first_frame_dispatched: false,
        });
    });

    // Dispatch liero-ready immediately — no async GPU init needed!
    if let Some(win) = web_sys::window() {
        let _ = win.dispatch_event(&web_sys::Event::new("liero-ready").unwrap());
    }
}

// ── Online multiplayer — relay integration ────────────────────────────────────

#[wasm_bindgen]
pub fn fetch_rooms(relay_wss_url: &str, callback: js_sys::Function) {
    let url = relay_wss_url.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        match do_fetch_rooms(&url).await {
            Ok(json) => {
                let _ = callback.call1(&JsValue::null(), &JsValue::from_str(&json));
            }
            Err(e) => {
                web_sys::console::error_1(&format!("[liero] fetch_rooms failed: {e}").into());
                let _ = callback.call1(&JsValue::null(), &JsValue::from_str("[]"));
            }
        }
    });
}

async fn do_fetch_rooms(url: &str) -> anyhow::Result<String> {
    let ws = web_sys::WebSocket::new(url).map_err(|e| anyhow!("{:?}", e))?;
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
    let open_promise = js_sys::Promise::new(&mut |resolve, _| {
        let cb = wasm_bindgen::closure::Closure::once(move || {
            let _ = resolve.call0(&JsValue::null());
        });
        ws.set_onopen(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    });
    wasm_bindgen_futures::JsFuture::from(open_promise)
        .await
        .map_err(|_| anyhow!("ws open"))?;
    ws.send_with_str("LIST")
        .map_err(|e| anyhow!("{:?}", e))?;
    let result: std::rc::Rc<RefCell<Option<String>>> = std::rc::Rc::new(RefCell::new(None));
    let result_cb = std::rc::Rc::clone(&result);
    let ws_close = ws.clone();
    let on_msg = wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
        if let Some(text) = e.data().as_string() {
            *result_cb.borrow_mut() = Some(text);
            ws_close.close().ok();
        }
    }) as Box<dyn FnMut(web_sys::MessageEvent)>);
    ws.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
    on_msg.forget();
    for _ in 0..100 {
        if result.borrow().is_some() {
            return Ok(result.borrow().clone().unwrap());
        }
        let p = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
                .ok();
        });
        wasm_bindgen_futures::JsFuture::from(p).await.ok();
    }
    Err(anyhow!("timeout"))
}

#[wasm_bindgen]
pub fn create_online_room(relay_wss_url: &str, name: &str) {
    let url = relay_wss_url.to_string();
    let name = name.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = do_create_room(&url, &name).await {
            web_sys::console::error_1(&format!("[liero] create_online_room: {e}").into());
        }
    });
}

async fn do_create_room(url: &str, name: &str) -> anyhow::Result<()> {
    let ws = open_ws_and_send(url, &format!("CREATE:{name}")).await?;
    let paired = handle_relay_handshake(ws, true).await?;
    JS_UI_EVENTS.with(|v| v.borrow_mut().push(UiAction::OnlineStart { local_player: 0 }));
    WS_SESSION.with(|s| *s.borrow_mut() = Some(paired));
    dispatch_event("liero-online-paired", None)?;
    Ok(())
}

#[wasm_bindgen]
pub fn join_online_room(relay_wss_url: &str, code: &str) {
    let url = relay_wss_url.to_string();
    let code = code.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = do_join_room(&url, &code).await {
            web_sys::console::error_1(&format!("[liero] join_online_room: {e}").into());
            dispatch_event("liero-online-error", Some(&format!("Join failed: {e}"))).ok();
        }
    });
}

async fn do_join_room(url: &str, code: &str) -> anyhow::Result<()> {
    let code = code.trim().to_uppercase();
    let ws = open_ws_and_send(url, &format!("JOIN:{code}")).await?;
    let session = handle_relay_handshake(ws, false).await?;
    JS_UI_EVENTS.with(|v| v.borrow_mut().push(UiAction::OnlineStart { local_player: 1 }));
    WS_SESSION.with(|s| *s.borrow_mut() = Some(session));
    dispatch_event("liero-online-paired", None)?;
    Ok(())
}

async fn open_ws_and_send(url: &str, msg: &str) -> anyhow::Result<web_sys::WebSocket> {
    let ws = web_sys::WebSocket::new(url).map_err(|e| anyhow!("{:?}", e))?;
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
    let open_promise = js_sys::Promise::new(&mut |resolve, _| {
        let cb = wasm_bindgen::closure::Closure::once(move || {
            let _ = resolve.call0(&JsValue::null());
        });
        ws.set_onopen(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    });
    wasm_bindgen_futures::JsFuture::from(open_promise)
        .await
        .map_err(|_| anyhow!("open"))?;
    ws.send_with_str(msg).map_err(|e| anyhow!("{:?}", e))?;
    Ok(ws)
}

async fn handle_relay_handshake(
    ws: web_sys::WebSocket,
    is_host: bool,
) -> anyhow::Result<WsSession> {
    let msg_queue: std::rc::Rc<RefCell<std::collections::VecDeque<String>>> =
        std::rc::Rc::new(RefCell::new(std::collections::VecDeque::new()));

    let q = std::rc::Rc::clone(&msg_queue);
    let on_msg = wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
        if let Some(text) = e.data().as_string() {
            q.borrow_mut().push_back(text);
        }
    }) as Box<dyn FnMut(web_sys::MessageEvent)>);
    ws.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
    on_msg.forget();

    loop {
        let p = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
                .ok();
        });
        wasm_bindgen_futures::JsFuture::from(p).await.ok();

        let msgs: Vec<String> = msg_queue.borrow_mut().drain(..).collect();
        for msg in msgs {
            let msg = msg.trim().to_string();
            if let Some(code) = msg.strip_prefix("ROOM:") {
                if is_host {
                    dispatch_event("liero-room-created", Some(code))?;
                }
            } else if msg == "PAIRED" {
                ws.set_onmessage(None);
                return Ok(WsSession::new(ws, if is_host { 0 } else { 1 }));
            } else if let Some(reason) = msg.strip_prefix("ERROR:") {
                return Err(anyhow!("relay error: {reason}"));
            }
        }
    }
}
