//! WebAssembly entry point for OpenLiero.
//!
//! Embeds the TC directory at compile time via `include_dir!`, initialises
//! a wgpu renderer (WebGL backend), runs a winit event loop in the browser,
//! and drives the same `liero-sim` game logic as the native desktop build.
//!
//! Build with:
//!   wasm-pack build openliero-rs/crates/liero-web --target web --out-dir ../../../web/pkg

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
// workspace_root/TC/openliero  ←  openliero-rs/crates/liero-web/../../../TC/openliero
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

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum WebGameState {
    Menu,
    Playing,
    GameOver(String),
}

/// State shared between the async wgpu init future and the winit event loop.
struct Inner {
    renderer:   Option<Renderer>,
    game_state: WebGameState,
    game:       Option<Game>,
    /// Raw input bits for P1 (idx 0) and P2 (idx 1).
    /// Bit layout: 0=left 1=right 2=up 3=down 4=fire 5=chgwpn 6=jump.
    inputs:     [u32; 4],
}

struct WebApp {
    tc:     Arc<Tc>,
    window: Option<Arc<Window>>,
    inner:  Arc<Mutex<Inner>>,
}

// ── Keyboard bit helpers ──────────────────────────────────────────────────────

fn key_bit_p1(code: KeyCode) -> Option<u32> {
    match code {
        KeyCode::ArrowLeft    => Some(0),
        KeyCode::ArrowRight   => Some(1),
        KeyCode::ArrowUp      => Some(2),
        KeyCode::ArrowDown    => Some(3),
        KeyCode::ControlLeft  => Some(4),
        KeyCode::ShiftLeft    => Some(5),
        KeyCode::Space        => Some(6),
        _ => None,
    }
}

fn key_bit_p2(code: KeyCode) -> Option<u32> {
    match code {
        KeyCode::KeyJ          => Some(0),
        KeyCode::KeyL          => Some(1),
        KeyCode::KeyI          => Some(2),
        KeyCode::KeyK          => Some(3),
        KeyCode::Enter         => Some(4),
        KeyCode::ShiftRight    => Some(5),
        KeyCode::ControlRight  => Some(6),
        _ => None,
    }
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
        // Avoid double-init on repeated resume events.
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("OpenLiero")
            .with_inner_size(PhysicalSize::new(640u32, 400u32));

        let window = Arc::new(
            event_loop.create_window(attrs).expect("create_window failed"),
        );
        self.window = Some(Arc::clone(&window));

        // Kick off async wgpu init.  `spawn_local` drives the future on the
        // JS microtask queue; the winit event loop continues normally.
        let tc    = Arc::clone(&self.tc);
        let inner = Arc::clone(&self.inner);

        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            match Renderer::new(Arc::clone(&window), &tc).await {
                Ok(renderer) => {
                    inner.lock().unwrap().renderer = Some(renderer);
                    web_sys::console::log_1(&"[liero] renderer ready".into());
                }
                Err(e) => {
                    web_sys::console::error_1(
                        &format!("[liero] renderer init failed: {e}").into(),
                    );
                }
            }
        });

        // On non-WASM (CI / cargo check), init synchronously so the crate
        // at least type-checks.  This path is never reached in production.
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
            // ── Lifecycle ──────────────────────────────────────────────────
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(r) = s.renderer.as_mut() {
                    r.resize(size);
                }
            }

            // ── Keyboard ───────────────────────────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key, state: ks, .. },
                ..
            } => {
                let pressed = ks == ElementState::Pressed;

                if let PhysicalKey::Code(code) = physical_key {
                    if let Some(bit) = key_bit_p1(code) {
                        if pressed { s.inputs[0] |= 1 << bit; }
                        else       { s.inputs[0] &= !(1 << bit); }
                    }
                    if let Some(bit) = key_bit_p2(code) {
                        if pressed { s.inputs[1] |= 1 << bit; }
                        else       { s.inputs[1] &= !(1 << bit); }
                    }

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
                                let mut game = Game::new(tc, 2, 42);
                                game.set_mode(GameMode::LastManStanding);
                                s.game       = Some(game);
                                s.game_state = WebGameState::Playing;
                                s.inputs     = [0; 4];
                            }
                            // Escape → back to menu.
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

            // ── Render frame ───────────────────────────────────────────────
            WindowEvent::RedrawRequested => {
                // ── 1. Step the game (no renderer involved) ─────────────
                let inputs = s.inputs; // [u32; 4] is Copy
                let game_over = if matches!(s.game_state, WebGameState::Playing) {
                    s.game.as_mut().and_then(|g| {
                        g.step(&inputs);
                        g.game_over()
                    })
                } else {
                    None
                };

                // ── 2. Transition to GameOver if needed ──────────────────
                if let Some(result) = game_over {
                    s.game_state = WebGameState::GameOver(format_result(result));
                }

                // ── 3. Render — destructure to borrow separate fields ────
                // Rust allows simultaneous borrows of distinct struct fields.
                let Inner { renderer, game_state, game, .. } = &mut *s;

                if let Some(r) = renderer.as_mut() {
                    match game_state {
                        WebGameState::Menu => {
                            r.render_menu(
                                "OPENLIERO",
                                &["PLAY LOCAL"],
                                0,
                                &[
                                    "PRESS ENTER OR SPACE TO PLAY",
                                    "P1: ARROWS + LCTRL / LSHIFT / SPACE",
                                    "P2: IJKL + ENTER / RSHIFT / RCTRL",
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

                // Schedule the next frame.
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
    // Redirect Rust panics to the browser console.
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

    // On WASM, `spawn_app` hands control back to the JS runtime immediately.
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }

    // On native (cargo check / CI), run synchronously.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).unwrap();
    }
}
