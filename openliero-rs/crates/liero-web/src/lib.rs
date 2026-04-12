//! WebAssembly entry point for OpenLiero.
//!
//! Embeds the TC directory at compile time via `include_dir!`, initialises
//! a wgpu renderer (WebGL backend), runs a winit event loop in the browser,
//! and drives the same `liero-sim` game logic as the native desktop build.
//!
//! Build with:
//!   wasm-pack build openliero-rs/crates/liero-web --target web --out-dir ../../../web/play/pkg

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use include_dir::{include_dir, Dir};
use liero_data::Tc;
use liero_render::Renderer;
use liero_sim::game::{Game, GameMode, GameResult};
use wasm_bindgen::prelude::*;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

// Embed TC at compile time.  Path is relative to this crate's Cargo.toml.
static TC_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../../TC/openliero");

// ── TC loading from embedded bytes ───────────────────────────────────────────

fn load_embedded_tc() -> anyhow::Result<Tc> {
    Tc::load_with(|rel_path| {
        TC_DIR
            .get_file(rel_path)
            .map(|f| f.contents().to_vec())
            .ok_or_else(|| anyhow!("embedded TC file not found: {rel_path}"))
    })
}

// ── Per-tab game configuration ────────────────────────────────────────────────
//
// Set from JS before the user starts a match (while the menu is shown).
// action bits: 0=left 1=right 2=up 3=down 4=fire 5=chgwpn 6=jump

#[derive(Clone)]
struct Config {
    player_count: usize,
    keymaps:      [[KeyCode; 7]; 4],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            player_count: 2,
            keymaps: [
                // P1 — arrows
                [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown,
                 KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::Space],
                // P2 — IJKL
                [KeyCode::KeyJ, KeyCode::KeyL, KeyCode::KeyI, KeyCode::KeyK,
                 KeyCode::Enter, KeyCode::ShiftRight, KeyCode::ControlRight],
                // P3 — WASD
                [KeyCode::KeyA, KeyCode::KeyD, KeyCode::KeyW, KeyCode::KeyS,
                 KeyCode::KeyQ, KeyCode::KeyE, KeyCode::Tab],
                // P4 — numpad
                [KeyCode::Numpad4, KeyCode::Numpad6, KeyCode::Numpad8, KeyCode::Numpad2,
                 KeyCode::Numpad0, KeyCode::NumpadDecimal, KeyCode::NumpadEnter],
            ],
        }
    }
}

thread_local! {
    static CONFIG: RefCell<Config> = RefCell::new(Config::default());
}

/// Parse a W3C `KeyboardEvent.code` string into a winit `KeyCode`.
fn str_to_keycode(s: &str) -> Option<KeyCode> {
    Some(match s {
        "ArrowLeft"      => KeyCode::ArrowLeft,
        "ArrowRight"     => KeyCode::ArrowRight,
        "ArrowUp"        => KeyCode::ArrowUp,
        "ArrowDown"      => KeyCode::ArrowDown,
        "ControlLeft"    => KeyCode::ControlLeft,
        "ControlRight"   => KeyCode::ControlRight,
        "ShiftLeft"      => KeyCode::ShiftLeft,
        "ShiftRight"     => KeyCode::ShiftRight,
        "AltLeft"        => KeyCode::AltLeft,
        "AltRight"       => KeyCode::AltRight,
        "Space"          => KeyCode::Space,
        "Enter"          => KeyCode::Enter,
        "NumpadEnter"    => KeyCode::NumpadEnter,
        "Tab"            => KeyCode::Tab,
        "Escape"         => KeyCode::Escape,
        "Backspace"      => KeyCode::Backspace,
        "Delete"         => KeyCode::Delete,
        "Insert"         => KeyCode::Insert,
        "Home"           => KeyCode::Home,
        "End"            => KeyCode::End,
        "PageUp"         => KeyCode::PageUp,
        "PageDown"       => KeyCode::PageDown,
        "KeyA"           => KeyCode::KeyA,
        "KeyB"           => KeyCode::KeyB,
        "KeyC"           => KeyCode::KeyC,
        "KeyD"           => KeyCode::KeyD,
        "KeyE"           => KeyCode::KeyE,
        "KeyF"           => KeyCode::KeyF,
        "KeyG"           => KeyCode::KeyG,
        "KeyH"           => KeyCode::KeyH,
        "KeyI"           => KeyCode::KeyI,
        "KeyJ"           => KeyCode::KeyJ,
        "KeyK"           => KeyCode::KeyK,
        "KeyL"           => KeyCode::KeyL,
        "KeyM"           => KeyCode::KeyM,
        "KeyN"           => KeyCode::KeyN,
        "KeyO"           => KeyCode::KeyO,
        "KeyP"           => KeyCode::KeyP,
        "KeyQ"           => KeyCode::KeyQ,
        "KeyR"           => KeyCode::KeyR,
        "KeyS"           => KeyCode::KeyS,
        "KeyT"           => KeyCode::KeyT,
        "KeyU"           => KeyCode::KeyU,
        "KeyV"           => KeyCode::KeyV,
        "KeyW"           => KeyCode::KeyW,
        "KeyX"           => KeyCode::KeyX,
        "KeyY"           => KeyCode::KeyY,
        "KeyZ"           => KeyCode::KeyZ,
        "Digit0"         => KeyCode::Digit0,
        "Digit1"         => KeyCode::Digit1,
        "Digit2"         => KeyCode::Digit2,
        "Digit3"         => KeyCode::Digit3,
        "Digit4"         => KeyCode::Digit4,
        "Digit5"         => KeyCode::Digit5,
        "Digit6"         => KeyCode::Digit6,
        "Digit7"         => KeyCode::Digit7,
        "Digit8"         => KeyCode::Digit8,
        "Digit9"         => KeyCode::Digit9,
        "F1"             => KeyCode::F1,
        "F2"             => KeyCode::F2,
        "F3"             => KeyCode::F3,
        "F4"             => KeyCode::F4,
        "F5"             => KeyCode::F5,
        "F6"             => KeyCode::F6,
        "F7"             => KeyCode::F7,
        "F8"             => KeyCode::F8,
        "F9"             => KeyCode::F9,
        "F10"            => KeyCode::F10,
        "F11"            => KeyCode::F11,
        "F12"            => KeyCode::F12,
        "Numpad0"        => KeyCode::Numpad0,
        "Numpad1"        => KeyCode::Numpad1,
        "Numpad2"        => KeyCode::Numpad2,
        "Numpad3"        => KeyCode::Numpad3,
        "Numpad4"        => KeyCode::Numpad4,
        "Numpad5"        => KeyCode::Numpad5,
        "Numpad6"        => KeyCode::Numpad6,
        "Numpad7"        => KeyCode::Numpad7,
        "Numpad8"        => KeyCode::Numpad8,
        "Numpad9"        => KeyCode::Numpad9,
        "NumpadDecimal"  => KeyCode::NumpadDecimal,
        "NumpadAdd"      => KeyCode::NumpadAdd,
        "NumpadSubtract" => KeyCode::NumpadSubtract,
        "NumpadMultiply" => KeyCode::NumpadMultiply,
        "NumpadDivide"   => KeyCode::NumpadDivide,
        "BracketLeft"    => KeyCode::BracketLeft,
        "BracketRight"   => KeyCode::BracketRight,
        "Semicolon"      => KeyCode::Semicolon,
        "Quote"          => KeyCode::Quote,
        "Backquote"      => KeyCode::Backquote,
        "Backslash"      => KeyCode::Backslash,
        "Slash"          => KeyCode::Slash,
        "Comma"          => KeyCode::Comma,
        "Period"         => KeyCode::Period,
        "Minus"          => KeyCode::Minus,
        "Equal"          => KeyCode::Equal,
        _                => return None,
    })
}

// ── JS-callable configuration API ────────────────────────────────────────────

/// Set how many players take part (2–4).
#[wasm_bindgen]
pub fn set_player_count(n: u32) {
    CONFIG.with(|c| {
        c.borrow_mut().player_count = (n as usize).clamp(2, 4);
    });
}

/// Bind a key to a player action.
///
/// * `player` — 0-based player index (0–3)
/// * `action` — action bit (0=left 1=right 2=up 3=down 4=fire 5=chgwpn 6=jump)
/// * `code`   — W3C `KeyboardEvent.code` string, e.g. `"ArrowLeft"`
///
/// Returns `true` if `code` was recognised, `false` otherwise.
#[wasm_bindgen]
pub fn set_player_key(player: u32, action: u32, code: &str) -> bool {
    let player = player as usize;
    let action = action as usize;
    if player >= 4 || action >= 7 {
        return false;
    }
    match str_to_keycode(code) {
        Some(kc) => {
            CONFIG.with(|c| c.borrow_mut().keymaps[player][action] = kc);
            true
        }
        None => false,
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum WebGameState {
    Menu,
    Playing,
    GameOver(String),
}

struct Inner {
    renderer:   Option<Renderer>,
    game_state: WebGameState,
    game:       Option<Game>,
    /// Raw input bits for each of the 4 possible players.
    /// Bit layout: 0=left 1=right 2=up 3=down 4=fire 5=chgwpn 6=jump.
    inputs:     [u32; 4],
}

struct WebApp {
    tc:     Arc<Tc>,
    window: Option<Arc<Window>>,
    inner:  Arc<Mutex<Inner>>,
}

fn format_result(r: GameResult) -> String {
    match r {
        GameResult::WormWins(i) => format!("PLAYER {} WINS!", i + 1),
        GameResult::TeamWins(t) => format!("TEAM {} WINS!", t + 1),
        GameResult::Draw        => "DRAW!".to_string(),
    }
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

impl ApplicationHandler for WebApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let mut attrs = Window::default_attributes()
            .with_title("OpenLiero")
            .with_inner_size(PhysicalSize::new(640u32, 400u32));

        // On WASM: attach to the existing <canvas id="canvas"> so winit renders
        // into the page's canvas instead of appending a new invisible one.
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;

            let canvas = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("canvas"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                .expect("canvas#canvas not found in DOM");

            attrs = attrs.with_canvas(Some(canvas));
        }

        let window = Arc::new(
            event_loop.create_window(attrs).expect("create_window failed"),
        );
        self.window = Some(Arc::clone(&window));

        let tc    = Arc::clone(&self.tc);
        let inner = Arc::clone(&self.inner);

        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            match Renderer::new(Arc::clone(&window), &tc).await {
                Ok(renderer) => {
                    inner.lock().unwrap().renderer = Some(renderer);
                    web_sys::console::log_1(&"[liero] renderer ready".into());

                    // Signal the page that we're ready
                    if let Some(win) = web_sys::window() {
                        let _ = win.dispatch_event(
                            &web_sys::Event::new("liero-ready").unwrap(),
                        );
                    }
                }
                Err(e) => {
                    web_sys::console::error_1(
                        &format!("[liero] renderer init failed: {e}").into(),
                    );
                }
            }
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let renderer = pollster::block_on(Renderer::new(Arc::clone(&window), &tc))
                .expect("renderer init");
            inner.lock().unwrap().renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _wid: WindowId,
        event: WindowEvent,
    ) {
        let mut s = self.inner.lock().unwrap();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(r) = s.renderer.as_mut() {
                    r.resize(size);
                }
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key, state: ks, .. },
                ..
            } => {
                let pressed = ks == ElementState::Pressed;

                if let PhysicalKey::Code(code) = physical_key {
                    // Update per-player input bits using the runtime keymap.
                    CONFIG.with(|cfg| {
                        let cfg = cfg.borrow();
                        for (pi, keymap) in cfg.keymaps[..cfg.player_count].iter().enumerate() {
                            for (bit, &bound) in keymap.iter().enumerate() {
                                if code == bound {
                                    if pressed { s.inputs[pi] |=  1 << bit; }
                                    else       { s.inputs[pi] &= !(1 << bit); }
                                }
                            }
                        }
                    });

                    if pressed {
                        match code {
                            // Enter/Space from Menu or GameOver → start new game.
                            KeyCode::Enter | KeyCode::Space
                                if matches!(
                                    s.game_state,
                                    WebGameState::Menu | WebGameState::GameOver(_)
                                ) =>
                            {
                                let tc = (*self.tc).clone();
                                let player_count = CONFIG.with(|c| c.borrow().player_count);
                                let mut game = Game::new(tc, player_count, 42);
                                game.set_mode(GameMode::LastManStanding);
                                s.game       = Some(game);
                                s.game_state = WebGameState::Playing;
                                s.inputs     = [0; 4];
                            }
                            KeyCode::Escape => {
                                s.game       = None;
                                s.game_state = WebGameState::Menu;
                                s.inputs     = [0; 4];
                            }
                            _ => {}
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                let inputs = s.inputs;
                let game_over = if matches!(s.game_state, WebGameState::Playing) {
                    s.game.as_mut().and_then(|g| {
                        g.step(&inputs);
                        g.game_over()
                    })
                } else {
                    None
                };

                if let Some(result) = game_over {
                    s.game_state = WebGameState::GameOver(format_result(result));
                }

                let Inner { renderer, game_state, game, .. } = &mut *s;

                if let Some(r) = renderer.as_mut() {
                    match game_state {
                        WebGameState::Menu => {
                            let player_count = CONFIG.with(|c| c.borrow().player_count);
                            let subtitle = match player_count {
                                2 => "2 PLAYERS",
                                3 => "3 PLAYERS",
                                4 => "4 PLAYERS",
                                _ => "2 PLAYERS",
                            };
                            r.render_menu(
                                "OPENLIERO",
                                &["PLAY LOCAL"],
                                0,
                                &[
                                    "PRESS ENTER OR SPACE TO START",
                                    subtitle,
                                ],
                            );
                        }
                        WebGameState::Playing => {
                            if let Some(g) = game.as_ref() {
                                r.render(g);
                            }
                        }
                        WebGameState::GameOver(msg) => {
                            r.render_game_over(msg);
                        }
                    }
                }

                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }

            _ => {}
        }
    }
}

// ── WASM entry point ──────────────────────────────────────────────────────────

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();

    web_sys::console::log_1(&"[liero] loading embedded TC…".into());

    let tc = match load_embedded_tc() {
        Ok(tc) => {
            web_sys::console::log_1(&"[liero] TC loaded".into());
            Arc::new(tc)
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[liero] TC load failed: {e}").into());
            return;
        }
    };

    let event_loop = EventLoop::new().expect("EventLoop::new");

    let app = WebApp {
        tc,
        window: None,
        inner: Arc::new(Mutex::new(Inner {
            renderer:   None,
            game_state: WebGameState::Menu,
            game:       None,
            inputs:     [0; 4],
        })),
    };

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).unwrap();
    }
}
