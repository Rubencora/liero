//! Native desktop entry point — winit window + wgpu renderer + liero-sim.
//!
//! # Controls — keyboard (hardcoded R1 defaults)
//!
//! | Key            | P1 action     | P2 key   | P2 action     |
//! |----------------|---------------|----------|---------------|
//! | ArrowLeft / A  | move left     | J        | move left     |
//! | ArrowRight / D | move right    | L        | move right    |
//! | ArrowUp / W    | aim up        | I        | aim up        |
//! | ArrowDown / S  | aim down      | K        | aim down      |
//! | LCtrl          | fire          | Enter    | fire          |
//! | LShift         | change weapon | RShift   | change weapon |
//! | Space          | jump          | RCtrl    | jump          |
//! | F2             | toggle CRT scanlines
//! | Escape         | back to menu (while playing)
//!
//! # Controls — gamepad (gilrs, up to 2 pads)
//!
//! Left stick / D-pad = move + aim,  RT/South = fire,
//! LB/West = change weapon,  B/RB = jump.
//!
//! Usage: openliero [TC_PATH]
//!        openliero [TC_PATH] --host-udp <PORT>
//!        openliero [TC_PATH] --connect-udp <ADDR:PORT>
//! Defaults from ~/.config/openliero/config.toml, TC default = TC/openliero.

mod config;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use config::Config;
use gilrs::{Axis, Button, Gilrs};
use liero_audio::AudioEngine;
use liero_net::RollbackSession;
use liero_render::Renderer;
use liero_sim::game::{Game, GameMode, GameResult};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

// ── Replay ───────────────────────────────────────────────────────────────────

/// Binary replay file format (little-endian throughout):
///   magic[4]  = b"LREP"
///   version   u32 = 1
///   seed      u32
///   num_worms u32
///   tc_hash   u64
///   mode      u32  (0=LMS, 1=TDM, 2=KotH, 3=BombTag, 4=Zombie, 5=Juggernaut, 6=DirtWar)
///   frames    N × [u32; 4]  (16 bytes per frame)
struct ReplayData {
    seed:      u32,
    num_worms: usize,
    tc_hash:   u64,
    mode:      u32,
    frames:    Vec<[u32; 4]>,
}

impl ReplayData {
    fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))?;
        let mut f = std::fs::File::create(path)?;
        f.write_all(b"LREP")?;
        f.write_all(&1u32.to_le_bytes())?;
        f.write_all(&self.seed.to_le_bytes())?;
        f.write_all(&(self.num_worms as u32).to_le_bytes())?;
        f.write_all(&self.tc_hash.to_le_bytes())?;
        f.write_all(&self.mode.to_le_bytes())?;
        for frame in &self.frames {
            for &v in frame.iter() { f.write_all(&v.to_le_bytes())?; }
        }
        Ok(())
    }

    fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let raw = std::fs::read(path)?;
        if raw.len() < 28 || &raw[..4] != b"LREP" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a LREP file"));
        }
        let _version  = u32::from_le_bytes(raw[4..8].try_into().unwrap());
        let seed      = u32::from_le_bytes(raw[8..12].try_into().unwrap());
        let num_worms = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        let tc_hash   = u64::from_le_bytes(raw[16..24].try_into().unwrap());
        let mode      = u32::from_le_bytes(raw[24..28].try_into().unwrap());
        let mut frames = Vec::new();
        let mut i = 28usize;
        while i + 16 <= raw.len() {
            let f = [
                u32::from_le_bytes(raw[i..i+4].try_into().unwrap()),
                u32::from_le_bytes(raw[i+4..i+8].try_into().unwrap()),
                u32::from_le_bytes(raw[i+8..i+12].try_into().unwrap()),
                u32::from_le_bytes(raw[i+12..i+16].try_into().unwrap()),
            ];
            frames.push(f);
            i += 16;
        }
        Ok(Self { seed, num_worms, tc_hash, mode, frames })
    }
}

struct PlaybackState {
    data:   ReplayData,
    cursor: usize,
    paused: bool,
}

fn replay_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("openliero")
        .join("last_replay.rep")
}

// ── Mode list ─────────────────────────────────────────────────────────────────

const MODE_NAMES: &[&str] = &[
    "LAST MAN STANDING",
    "TEAM DEATHMATCH",
    "KING OF THE HILL",
    "BOMB TAG",
    "ZOMBIE MODE",
    "JUGGERNAUT",
    "DIRT WAR",
];

fn mode_from_idx(idx: usize) -> GameMode {
    match idx {
        0 => GameMode::LastManStanding,
        1 => GameMode::TeamDeathmatch,
        2 => GameMode::KingOfHill { time_limit: 1800 },
        3 => GameMode::BombTag    { fuse: 600 },
        4 => GameMode::ZombieMode,
        5 => GameMode::Juggernaut,
        6 => GameMode::DirtWar    { time_limit: 18000 }, // 5 min @ 60 fps
        _ => GameMode::LastManStanding,
    }
}

fn mode_to_u32(mode: &GameMode) -> u32 {
    match mode {
        GameMode::LastManStanding    => 0,
        GameMode::TeamDeathmatch     => 1,
        GameMode::KingOfHill { .. }  => 2,
        GameMode::BombTag    { .. }  => 3,
        GameMode::ZombieMode         => 4,
        GameMode::Juggernaut         => 5,
        GameMode::DirtWar    { .. }  => 6,
    }
}

// ── Game state machine ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum GameState {
    MainMenu   { cursor: usize },
    LocalSetup { mode_idx: usize },
    /// Direct-IP host mode (advanced): just listen on a port.
    OnlineHost { port_str: String },
    /// Relay host mode: background thread handles relay, shows room code.
    RelayHost {
        /// Room code shown to user once relay responds; None = still connecting.
        room_code: Arc<Mutex<Option<String>>>,
        /// Joiner's public IP once they join; None = still waiting.
        joiner_result: Arc<Mutex<Option<Result<std::net::IpAddr, String>>>>,
        /// The UDP port we're listening on.
        udp_port: u16,
    },
    /// Relay join mode: user types 6-letter code.
    RelayJoin { code: String },
    /// Direct-IP join mode (advanced): user types IP:PORT.
    OnlineJoin { addr_str: String },
    Playing,
    GameOver   { msg: String },
}

const MAIN_OPTIONS: &[&str] = &["PLAY LOCAL", "HOST ONLINE", "JOIN ONLINE", "QUIT"];

// ── Argument parsing ──────────────────────────────────────────────────────────

/// Resolve the TC directory relative to the running executable.
///
/// Search order:
/// 1. `TC/openliero/` next to the binary  (Windows, Linux, bare macOS binary)
/// 2. `../Resources/TC/openliero/` relative to the binary (macOS .app bundle:
///    `Contents/MacOS/openliero` → `Contents/Resources/TC/openliero`)
/// 3. `TC/openliero` relative to the current working directory (dev / fallback)
fn default_tc_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let sibling = bin_dir.join("TC/openliero");
            if sibling.is_dir() {
                return sibling;
            }
            // macOS .app: Contents/MacOS → Contents/Resources
            if let Some(contents) = bin_dir.parent() {
                let bundle = contents.join("Resources/TC/openliero");
                if bundle.is_dir() {
                    return bundle;
                }
            }
        }
    }
    PathBuf::from("TC/openliero")
}

fn parse_tc_path(config: &Config) -> PathBuf {
    // Explicit CLI argument wins.
    if let Some(arg) = std::env::args().nth(1).filter(|a| !a.starts_with('-')) {
        return PathBuf::from(arg);
    }
    // User-customised config wins over the built-in default.
    if config.tc_path != "TC/openliero" {
        return PathBuf::from(&config.tc_path);
    }
    // Otherwise resolve relative to the executable (supports .app bundles, etc.).
    default_tc_path()
}

fn main() -> Result<()> {
    let config     = Config::load();
    let tc_path    = parse_tc_path(&config);
    let event_loop = EventLoop::new()?;
    let mut app    = App::new(tc_path, config);
    event_loop.run_app(&mut app)?;
    Ok(())
}

// ── Application state ─────────────────────────────────────────────────────────

struct App {
    tc_path:    PathBuf,
    config:     Config,
    game_state: GameState,
    inner:      Option<AppInner>,
}

struct AppInner {
    window:       Arc<Window>,
    renderer:     Renderer,
    audio:        AudioEngine,
    gilrs:        Gilrs,
    /// Keyboard input bits for each player slot.
    kb_inputs:    [u32; 4],
    // Game-specific (None while in menu).
    game:         Option<Game>,
    net:          Option<RollbackSession>,
    local_player: usize,
    /// True while recording inputs.
    recording:     bool,
    /// Inputs recorded so far (one [u32;4] per frame).
    replay_frames: Vec<[u32; 4]>,
    /// Active playback session, if any.
    playback:      Option<PlaybackState>,
}

impl App {
    fn new(tc_path: PathBuf, config: Config) -> Self {
        let last_mode = config.last_mode;
        Self {
            tc_path,
            config,
            game_state: GameState::LocalSetup { mode_idx: last_mode }, // pre-select last mode
            inner: None,
        }
    }

    // ── Game lifecycle ────────────────────────────────────────────────────────

    /// Load TC from disk and apply `mod.lua` if present in the TC directory.
    fn load_tc(&self) -> liero_data::Tc {
        let mut tc = liero_data::Tc::load(&self.tc_path)
            .unwrap_or_else(|e| panic!("failed to load TC: {e}"));
        let mod_path = self.tc_path.join("mod.lua");
        if mod_path.exists() {
            match std::fs::read_to_string(&mod_path) {
                Ok(src) => {
                    if let Err(e) = liero_mod::apply_mod(&mut tc, &src) {
                        eprintln!("[mod] mod.lua error: {e}");
                    } else {
                        eprintln!("[mod] applied mod.lua from {:?}", mod_path);
                    }
                }
                Err(e) => eprintln!("[mod] could not read mod.lua: {e}"),
            }
        }
        tc
    }

    fn start_local_game(&mut self, mode_idx: usize) {
        let tc = self.load_tc();
        let mut game = Game::new(tc, 2, 42);
        game.set_mode(mode_from_idx(mode_idx));

        if let Some(inner) = &mut self.inner {
            inner.game         = Some(game);
            inner.net          = None;
            inner.local_player = 0;
            inner.kb_inputs    = [0; 4];
        }
        self.config.last_mode = mode_idx;
        self.config.save();
        self.game_state = GameState::Playing;
    }

    fn start_host_game(&mut self, port_str: &str) {
        let port: u16 = port_str.parse().unwrap_or(7777);
        let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();

        let tc  = self.load_tc();
        let tc_hash = liero_net::tc_hash_of(&tc);
        let game = Game::new(tc, 2, 42);
        let net  = RollbackSession::host(addr, 0)
            .map(|mut s| {
                s.set_tc_hash(tc_hash);
                eprintln!("[net] hosting on port {port}, waiting for peer... (tc_hash={tc_hash:016x})");
                s
            })
            .map_err(|e| eprintln!("[net] host failed: {e}"))
            .ok();

        if let Some(inner) = &mut self.inner {
            inner.game         = Some(game);
            inner.net          = net;
            inner.local_player = 0;
            inner.kb_inputs    = [0; 4];
        }
        self.config.host_port = port_str.to_string();
        self.config.save();
        self.game_state = GameState::Playing;
    }

    fn start_join_game(&mut self, addr_str: &str) {
        let remote: SocketAddr = addr_str.parse()
            .unwrap_or_else(|_| "127.0.0.1:7777".parse().unwrap());
        let local: SocketAddr  = "0.0.0.0:0".parse().unwrap();

        let tc   = self.load_tc();
        let tc_hash = liero_net::tc_hash_of(&tc);
        let game = Game::new(tc, 2, 42);
        let (net, local_player) = RollbackSession::connect(local, remote, 1)
            .map(|mut s| {
                s.set_tc_hash(tc_hash);
                eprintln!("[net] connecting to {remote}... (tc_hash={tc_hash:016x})");
                (Some(s), 1usize)
            })
            .unwrap_or_else(|e| { eprintln!("[net] connect failed: {e}"); (None, 0) });

        if let Some(inner) = &mut self.inner {
            inner.game         = Some(game);
            inner.net          = net;
            inner.local_player = local_player;
            inner.kb_inputs    = [0; 4];
        }
        self.game_state = GameState::Playing;
    }

    fn start_relay_host(&mut self) {
        let port: u16 = self.config.host_port.parse().unwrap_or(7777);
        let bind_addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();

        // Pre-open UDP socket so port is ready when peer connects.
        let tc = self.load_tc();
        let tc_hash = liero_net::tc_hash_of(&tc);
        let game = Game::new(tc.clone(), 2, 42);
        let net = RollbackSession::host(bind_addr, 0)
            .map(|mut s| {
                s.set_tc_hash(tc_hash);
                s
            })
            .ok();

        if let Some(inner) = &mut self.inner {
            inner.game         = Some(game);
            inner.net          = net;
            inner.local_player = 0;
            inner.kb_inputs    = [0; 4];
        }

        let room_code: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let joiner_result: Arc<Mutex<Option<Result<std::net::IpAddr, String>>>> = Arc::new(Mutex::new(None));

        let code_clone   = Arc::clone(&room_code);
        let result_clone = Arc::clone(&joiner_result);
        let relay_addr   = self.config.relay_url.clone();

        std::thread::spawn(move || {
            match liero_net::relay_host_split(&relay_addr, port, |code| {
                *code_clone.lock().unwrap() = Some(code);
            }) {
                Ok(ip) => *result_clone.lock().unwrap() = Some(Ok(ip)),
                Err(e) => {
                    *code_clone.lock().unwrap() = Some("ERROR".to_string());
                    *result_clone.lock().unwrap() = Some(Err(e.to_string()));
                }
            }
        });

        self.game_state = GameState::RelayHost { room_code, joiner_result, udp_port: port };
    }

    fn start_relay_join(&mut self, code: &str) {
        let relay = self.config.relay_url.clone();
        let code = code.to_string();

        match liero_net::relay_join(&relay, &code) {
            Ok(peer_addr) => {
                let local: SocketAddr = "0.0.0.0:0".parse().unwrap();
                let tc = self.load_tc();
                let tc_hash = liero_net::tc_hash_of(&tc);
                let game = Game::new(tc, 2, 42);
                let (net, local_player) = RollbackSession::connect(local, peer_addr, 1)
                    .map(|mut s| {
                        s.set_tc_hash(tc_hash);
                        eprintln!("[net] joining relay peer {peer_addr}... (tc_hash={tc_hash:016x})");
                        (Some(s), 1usize)
                    })
                    .unwrap_or_else(|e| { eprintln!("[net] connect failed: {e}"); (None, 0) });

                if let Some(inner) = &mut self.inner {
                    inner.game         = Some(game);
                    inner.net          = net;
                    inner.local_player = local_player;
                    inner.kb_inputs    = [0; 4];
                }
                self.game_state = GameState::Playing;
            }
            Err(e) => {
                eprintln!("[relay] join failed: {e}");
                self.game_state = GameState::MainMenu { cursor: 0 };
            }
        }
    }

    fn quit_to_menu(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.game      = None;
            inner.net       = None;
            inner.kb_inputs = [0; 4];
            inner.recording = false;
            inner.replay_frames.clear();
            inner.playback  = None;
        }
        self.game_state = GameState::MainMenu { cursor: 0 };
    }

    // ── Menu drawing ──────────────────────────────────────────────────────────

    fn draw_current_menu(&mut self) {
        let state = self.game_state.clone();
        let Some(inner) = self.inner.as_mut() else { return };
        match state {
            GameState::MainMenu { cursor } => {
                inner.renderer.render_menu(
                    "OPENLIERO",
                    MAIN_OPTIONS,
                    cursor,
                    &["UP/DOWN + ENTER TO SELECT  |  ESC TO QUIT"],
                );
            }
            GameState::LocalSetup { mode_idx } => {
                inner.renderer.render_menu(
                    "SELECT MODE",
                    MODE_NAMES,
                    mode_idx,
                    &["ENTER = START   |   ESC = BACK"],
                );
            }
            GameState::OnlineHost { ref port_str } => {
                let label = format!("PORT: {port_str}_");
                inner.renderer.render_menu(
                    "HOST GAME",
                    &[&label],
                    0,
                    &["TYPE PORT NUMBER", "ENTER = START   |   ESC = BACK"],
                );
            }
            GameState::RelayHost { ref room_code, ref joiner_result, udp_port: _ } => {
                let code_display = room_code.lock().unwrap()
                    .clone()
                    .unwrap_or_else(|| "......".to_string());
                let status = if joiner_result.lock().unwrap().is_some() {
                    "PEER FOUND — starting...".to_string()
                } else if code_display == "ERROR" {
                    "RELAY ERROR — press ESC".to_string()
                } else {
                    "WAITING FOR PLAYER 2...".to_string()
                };
                inner.renderer.render_menu(
                    "HOST GAME",
                    &[&format!("CODE: {code_display}"), &status],
                    0,
                    &["SHARE THIS CODE WITH YOUR FRIEND", "ESC = CANCEL"],
                );
            }
            GameState::RelayJoin { ref code } => {
                let display = if code.is_empty() { "_".to_string() } else { format!("{}_", code) };
                inner.renderer.render_menu(
                    "JOIN GAME",
                    &[&format!("CODE: {display}")],
                    0,
                    &["TYPE THE 6-LETTER CODE FROM HOST", "ENTER = CONNECT   |   ESC = BACK"],
                );
            }
            GameState::OnlineJoin { ref addr_str } => {
                let label = format!("ADDRESS: {addr_str}_");
                inner.renderer.render_menu(
                    "JOIN GAME",
                    &[&label],
                    0,
                    &["TYPE IP:PORT  (eg. 1.2.3.4:7777)", "ENTER = CONNECT   |   ESC = BACK"],
                );
            }
            GameState::GameOver { ref msg } => {
                inner.renderer.render_game_over(msg);
            }
            GameState::Playing => {}
        }
    }
}

// ── Gamepad polling ───────────────────────────────────────────────────────────

/// Poll up to 2 connected gamepads.
/// Returns raw input bits for player slots 0 and 1.
/// Bit layout mirrors keyboard: 0=left 1=right 2=up 3=down 4=fire 5=chgwpn 6=jump.
fn poll_gamepads(gilrs: &mut Gilrs) -> [u32; 2] {
    // Drain internal event queue so axis/button state is up to date.
    while gilrs.next_event().is_some() {}

    let mut out = [0u32; 2];
    for (slot, (_, gp)) in gilrs.gamepads().enumerate().take(2) {
        let ax = gp.value(Axis::LeftStickX);
        let ay = gp.value(Axis::LeftStickY);
        let dpx = if gp.is_pressed(Button::DPadRight) { 1.0f32 }
                  else if gp.is_pressed(Button::DPadLeft) { -1.0 }
                  else { 0.0 };
        let dpy = if gp.is_pressed(Button::DPadDown) { 1.0f32 }
                  else if gp.is_pressed(Button::DPadUp) { -1.0 }
                  else { 0.0 };
        let x = ax + dpx;
        let y = ay + dpy;
        let mut b = 0u32;
        if x < -0.3 { b |= 1 << 0; }  // left
        if x >  0.3 { b |= 1 << 1; }  // right
        if y < -0.3 { b |= 1 << 2; }  // up
        if y >  0.3 { b |= 1 << 3; }  // down
        // Fire: right trigger or South (A/Cross)
        if gp.is_pressed(Button::RightTrigger2) || gp.is_pressed(Button::South) {
            b |= 1 << 4;
        }
        // Change weapon: left bumper or West (X/Square)
        if gp.is_pressed(Button::LeftTrigger) || gp.is_pressed(Button::West) {
            b |= 1 << 5;
        }
        // Jump: B/Circle or right bumper
        if gp.is_pressed(Button::East) || gp.is_pressed(Button::RightTrigger) {
            b |= 1 << 6;
        }
        out[slot] = b;
    }
    out
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.inner.is_some() { return; }

        let attrs = Window::default_attributes()
            .with_title("OpenLiero")
            .with_inner_size(PhysicalSize::new(640u32, 400u32));

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[fatal] cannot create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let tc = self.load_tc();

        let mut renderer = match pollster::block_on(Renderer::new(Arc::clone(&window), &tc)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[fatal] wgpu renderer init failed: {e}");
                eprintln!("[fatal] Your macOS version may not be supported by the bundled wgpu version.");
                eprintln!("[fatal] Try updating the application or running from source with a newer wgpu.");
                event_loop.exit();
                return;
            }
        };
        renderer.set_scanlines(self.config.scanlines);

        let audio = AudioEngine::new(tc.sounds.clone())
            .unwrap_or_else(|e| {
                eprintln!("[audio] init failed (no sound): {e}");
                AudioEngine::new(Vec::new()).expect("silent audio engine")
            });

        let gilrs = Gilrs::new().unwrap_or_else(|e| {
            eprintln!("[gamepad] gilrs init failed (no gamepad support): {e}");
            Gilrs::new().expect("second gilrs attempt")
        });

        window.request_redraw();

        self.inner = Some(AppInner {
            window,
            renderer,
            audio,
            gilrs,
            kb_inputs:    [0; 4],
            game:         None,
            net:          None,
            local_player: 0,
            recording:     false,
            replay_frames: Vec::new(),
            playback:      None,
        });

        // Show the pre-selected state (LocalSetup with last_mode).
        self.draw_current_menu();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(inner) = self.inner.as_mut() else { return };

        match event {
            WindowEvent::CloseRequested => {
                self.config.save();
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                inner.renderer.resize(size);
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    physical_key: PhysicalKey::Code(code),
                    logical_key:  lk,
                    state,
                    ..
                },
                ..
            } => {
                // F2: toggle scanlines (any state).
                if code == KeyCode::F2 && state == ElementState::Pressed {
                    let on = !inner.renderer.crt_scanlines;
                    inner.renderer.set_scanlines(on);
                    self.config.scanlines = on;
                    self.config.save();
                    return;
                }

                match self.game_state.clone() {
                    GameState::Playing => {
                        if state != ElementState::Pressed {
                            let inner = self.inner.as_mut().unwrap();
                            update_inputs(&mut inner.kb_inputs, code, state);
                            return;
                        }
                        match code {
                            KeyCode::Escape => {
                                self.quit_to_menu();
                                self.draw_current_menu();
                            }
                            KeyCode::F5 => {
                                let inner = self.inner.as_mut().unwrap();
                                if inner.recording {
                                    // Stop recording, save replay.
                                    inner.recording = false;
                                    let game = inner.game.as_ref().unwrap();
                                    let rep = ReplayData {
                                        seed:      game.seed,
                                        num_worms: game.worms.len(),
                                        tc_hash:   liero_net::tc_hash_of(&game.tc),
                                        mode:      mode_to_u32(&game.mode),
                                        frames:    std::mem::take(&mut inner.replay_frames),
                                    };
                                    let p = replay_path();
                                    match rep.save(&p) {
                                        Ok(()) => eprintln!("[replay] saved to {:?}", p),
                                        Err(e) => eprintln!("[replay] save failed: {e}"),
                                    }
                                } else {
                                    // Start recording.
                                    inner.recording = true;
                                    inner.replay_frames.clear();
                                    eprintln!("[replay] recording started");
                                }
                            }
                            KeyCode::F7 => {
                                // Load and start playback.
                                let p = replay_path();
                                match ReplayData::load(&p) {
                                    Ok(data) => {
                                        // Load TC before borrowing inner mutably
                                        let tc = self.load_tc();
                                        let nw = data.num_worms;
                                        let seed = data.seed;
                                        let mode = data.mode as usize;
                                        let mut game = Game::new(tc, nw, seed);
                                        game.set_mode(mode_from_idx(mode));

                                        let inner = self.inner.as_mut().unwrap();
                                        let pb = PlaybackState { data, cursor: 0, paused: false };
                                        inner.game     = Some(game);
                                        inner.playback = Some(pb);
                                        inner.net      = None;
                                        eprintln!("[replay] playback started");
                                    }
                                    Err(e) => eprintln!("[replay] load failed: {e}"),
                                }
                            }
                            _ => {
                                let inner = self.inner.as_mut().unwrap();
                                // Handle playback controls
                                if let Some(pb) = &mut inner.playback {
                                    match code {
                                        KeyCode::Space => { pb.paused = !pb.paused; }
                                        KeyCode::ArrowRight if pb.paused => {
                                            if pb.cursor < pb.data.frames.len() {
                                                let saved = pb.data.frames[pb.cursor];
                                                inner.game.as_mut().unwrap().step(&saved);
                                                pb.cursor += 1;
                                            }
                                        }
                                        KeyCode::ArrowLeft if pb.paused && pb.cursor > 0 => {
                                            // Rewind: collect params without holding mutable ref
                                            let target = pb.cursor - 1;
                                            let seed = pb.data.seed;
                                            let nw   = pb.data.num_worms;
                                            let mode = pb.data.mode as usize;
                                            let frames_copy = pb.data.frames.clone();
                                            pb.cursor = target;
                                            // Drop refs by moving out of scope
                                            let _ = pb;
                                            let _ = inner;

                                            // Now we can call load_tc without conflicts
                                            let tc = self.load_tc();
                                            let mut game = Game::new(tc, nw, seed);
                                            game.set_mode(mode_from_idx(mode));
                                            for f in 0..target {
                                                game.step(&frames_copy[f]);
                                            }
                                            self.inner.as_mut().unwrap().game = Some(game);
                                        }
                                        _ => update_inputs(&mut inner.kb_inputs, code, state),
                                    }
                                } else {
                                    update_inputs(&mut inner.kb_inputs, code, state);
                                }
                            }
                        }
                    }

                    GameState::MainMenu { cursor } => {
                        if state != ElementState::Pressed { return; }
                        let n = MAIN_OPTIONS.len();
                        match code {
                            KeyCode::ArrowUp   | KeyCode::KeyW => {
                                self.game_state = GameState::MainMenu { cursor: (cursor + n - 1) % n };
                            }
                            KeyCode::ArrowDown | KeyCode::KeyS => {
                                self.game_state = GameState::MainMenu { cursor: (cursor + 1) % n };
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter => match cursor {
                                0 => self.game_state = GameState::LocalSetup {
                                    mode_idx: self.config.last_mode,
                                },
                                1 => {
                                    // HOST ONLINE — start relay host
                                    self.start_relay_host();
                                    self.draw_current_menu();
                                    return;
                                },
                                2 => {
                                    // JOIN ONLINE — show relay code input
                                    self.game_state = GameState::RelayJoin {
                                        code: String::new(),
                                    };
                                },
                                3 => { self.config.save(); event_loop.exit(); return; }
                                _ => {}
                            },
                            KeyCode::Escape => { self.config.save(); event_loop.exit(); return; }
                            _ => {}
                        }
                        self.draw_current_menu();
                    }

                    GameState::LocalSetup { mode_idx } => {
                        if state != ElementState::Pressed { return; }
                        let n = MODE_NAMES.len();
                        match code {
                            KeyCode::ArrowUp   | KeyCode::KeyW => {
                                self.game_state = GameState::LocalSetup {
                                    mode_idx: (mode_idx + n - 1) % n,
                                };
                                self.draw_current_menu();
                            }
                            KeyCode::ArrowDown | KeyCode::KeyS => {
                                self.game_state = GameState::LocalSetup {
                                    mode_idx: (mode_idx + 1) % n,
                                };
                                self.draw_current_menu();
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                self.start_local_game(mode_idx);
                                self.inner.as_ref().unwrap().window.request_redraw();
                            }
                            KeyCode::Escape => {
                                self.game_state = GameState::MainMenu { cursor: 0 };
                                self.draw_current_menu();
                            }
                            _ => {}
                        }
                    }

                    GameState::OnlineHost { port_str } => {
                        if state != ElementState::Pressed { return; }
                        let mut port = port_str.clone();
                        match code {
                            KeyCode::Backspace => { port.pop(); }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                let ps = port.clone();
                                self.start_host_game(&ps);
                                self.inner.as_ref().unwrap().window.request_redraw();
                                return;
                            }
                            KeyCode::Escape => {
                                self.game_state = GameState::MainMenu { cursor: 0 };
                                self.draw_current_menu();
                                return;
                            }
                            _ => {
                                if let Key::Character(ch) = &lk {
                                    if let Some(c) = ch.chars().next() {
                                        if c.is_ascii_digit() && port.len() < 5 {
                                            port.push(c);
                                        }
                                    }
                                }
                            }
                        }
                        self.game_state = GameState::OnlineHost { port_str: port };
                        self.draw_current_menu();
                    }

                    GameState::RelayHost { .. } => {
                        if state != ElementState::Pressed { return; }
                        match code {
                            KeyCode::Escape => {
                                self.quit_to_menu();
                                self.draw_current_menu();
                            }
                            _ => {}
                        }
                    }

                    GameState::RelayJoin { code: code_str } => {
                        if state != ElementState::Pressed { return; }
                        let mut c = code_str.clone();
                        match code {
                            KeyCode::Backspace => { c.pop(); }
                            KeyCode::Enter | KeyCode::NumpadEnter if c.len() == 6 => {
                                let code = c.to_uppercase();
                                self.start_relay_join(&code);
                                self.inner.as_ref().unwrap().window.request_redraw();
                                return;
                            }
                            KeyCode::Escape => {
                                self.game_state = GameState::MainMenu { cursor: 0 };
                                self.draw_current_menu();
                                return;
                            }
                            _ => {
                                if let Key::Character(ch) = &lk {
                                    if let Some(ch) = ch.chars().next() {
                                        if ch.is_ascii_alphabetic() && c.len() < 6 {
                                            c.push(ch.to_ascii_uppercase());
                                        }
                                    }
                                }
                            }
                        }
                        self.game_state = GameState::RelayJoin { code: c };
                        self.draw_current_menu();
                    }

                    GameState::OnlineJoin { addr_str } => {
                        if state != ElementState::Pressed { return; }
                        let mut addr = addr_str.clone();
                        match code {
                            KeyCode::Backspace => { addr.pop(); }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                let a = addr.clone();
                                self.start_join_game(&a);
                                self.inner.as_ref().unwrap().window.request_redraw();
                                return;
                            }
                            KeyCode::Escape => {
                                self.game_state = GameState::MainMenu { cursor: 0 };
                                self.draw_current_menu();
                                return;
                            }
                            _ => {
                                if let Key::Character(ch) = &lk {
                                    if let Some(c) = ch.chars().next() {
                                        if (c.is_ascii_alphanumeric() || c == '.' || c == ':')
                                            && addr.len() < 21
                                        {
                                            addr.push(c);
                                        }
                                    }
                                }
                            }
                        }
                        self.game_state = GameState::OnlineJoin { addr_str: addr };
                        self.draw_current_menu();
                    }

                    GameState::GameOver { .. } => {
                        if state == ElementState::Pressed
                            && matches!(
                                code,
                                KeyCode::Enter | KeyCode::NumpadEnter
                                    | KeyCode::Space
                                    | KeyCode::Escape
                            )
                        {
                            self.quit_to_menu();
                            self.draw_current_menu();
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Check relay host: transition to Playing when peer joins.
                if let GameState::RelayHost { ref joiner_result, .. } = self.game_state {
                    if joiner_result.lock().unwrap().is_some() {
                        self.game_state = GameState::Playing;
                    } else {
                        // Still waiting; request redraw to show updates
                        self.inner.as_ref().unwrap().window.request_redraw();
                        self.draw_current_menu();
                        return;
                    }
                }

                if let GameState::Playing = self.game_state {
                    let inner = self.inner.as_mut().unwrap();
                    let game  = inner.game.as_mut().unwrap();

                    // Poll gamepads and merge with keyboard inputs.
                    let gp = poll_gamepads(&mut inner.gilrs);
                    let mut eff = inner.kb_inputs;
                    eff[0] |= gp[0];
                    eff[1] |= gp[1];

                    // Simulation tick.
                    if let Some(pb) = &mut inner.playback {
                        // Replay playback mode — feed stored inputs.
                        if !pb.paused {
                            if pb.cursor < pb.data.frames.len() {
                                let saved = pb.data.frames[pb.cursor];
                                game.step(&saved);
                                pb.cursor += 1;
                            }
                            // When replay ends, leave game frozen (user can quit with Esc).
                        }
                    } else if let Some(net) = &mut inner.net {
                        let local_raw = eff[inner.local_player];
                        net.step(game, local_raw);
                        if net.desync_detected {
                            eprintln!("[net] DESYNC at frame {}", net.desync_frame);
                        }
                    } else {
                        // Record before stepping.
                        if inner.recording {
                            inner.replay_frames.push(eff);
                        }
                        game.step(&eff);
                    }

                    // Audio events.
                    for ev in &game.sound_events {
                        use liero_sim::game::SoundEvent;
                        let idx = match ev {
                            SoundEvent::WeaponFire { weapon_idx } => {
                                game.tc.weapons.get(*weapon_idx)
                                    .filter(|w| w.launch_sound >= 0)
                                    .map(|w| w.launch_sound as usize)
                                    .filter(|&s| s < game.tc.sounds.len())
                            }
                            SoundEvent::Explosion { sobj_idx } => {
                                game.tc.sobjects.get(*sobj_idx)
                                    .filter(|s| s.start_sound >= 0)
                                    .map(|s| s.start_sound as usize)
                            }
                            SoundEvent::Hurt  { .. } => Some(18),
                            SoundEvent::Death { .. } => Some(15),
                            SoundEvent::BonusCollect  => Some(21),
                        };
                        if let Some(i) = idx { inner.audio.play(i); }
                    }

                    // Render.
                    inner.renderer.render(game);
                    inner.window.request_redraw();

                    // Game over check.
                    if let Some(result) = game.game_over() {
                        let msg = match result {
                            GameResult::WormWins(i) => format!("WORM {} WINS!", i + 1),
                            GameResult::TeamWins(t) => format!("TEAM {} WINS!", t + 1),
                            GameResult::Draw        => "DRAW!".to_string(),
                        };
                        self.game_state = GameState::GameOver { msg };
                        self.draw_current_menu();
                    }
                } else {
                    self.draw_current_menu();
                }
            }

            _ => {}
        }
    }
}

// ── Input mapping ─────────────────────────────────────────────────────────────

fn key_binding(code: KeyCode) -> Option<(usize, u32)> {
    match code {
        KeyCode::ArrowLeft  | KeyCode::KeyA => Some((0, 1 << 0)),
        KeyCode::ArrowRight | KeyCode::KeyD => Some((0, 1 << 1)),
        KeyCode::ArrowUp    | KeyCode::KeyW => Some((0, 1 << 2)),
        KeyCode::ArrowDown  | KeyCode::KeyS => Some((0, 1 << 3)),
        KeyCode::ControlLeft                => Some((0, 1 << 4)),
        KeyCode::ShiftLeft                  => Some((0, 1 << 5)),
        KeyCode::Space                      => Some((0, 1 << 6)),
        KeyCode::KeyJ                       => Some((1, 1 << 0)),
        KeyCode::KeyL                       => Some((1, 1 << 1)),
        KeyCode::KeyI                       => Some((1, 1 << 2)),
        KeyCode::KeyK                       => Some((1, 1 << 3)),
        KeyCode::Enter                      => Some((1, 1 << 4)),
        KeyCode::ShiftRight                 => Some((1, 1 << 5)),
        KeyCode::ControlRight               => Some((1, 1 << 6)),
        _                                   => None,
    }
}

fn update_inputs(inputs: &mut [u32; 4], code: KeyCode, state: ElementState) {
    if let Some((player, bit)) = key_binding(code) {
        match state {
            ElementState::Pressed  => inputs[player] |=  bit,
            ElementState::Released => inputs[player] &= !bit,
        }
    }
}
