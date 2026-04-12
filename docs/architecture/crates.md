# Crate Architecture

OpenLiero is a Cargo workspace. All crates live under `openliero-rs/`.

## Crate dependency graph

```
liero-desktop ──┬── liero-render ──── liero-data
                ├── liero-sim    ──── liero-data
                ├── liero-audio  ──── liero-data
                ├── liero-net    ──── liero-data
                └── liero-mod    ──── liero-data

liero-web    ───┬── liero-render
                ├── liero-sim
                └── liero-data  (embedded via include_dir)

tools/liero-relay    (standalone, no workspace crate deps)
tools/tc-validator ──── liero-data
tools/replay-diff  ──── liero-sim, liero-data
```

## Crate summaries

### `liero-data`

Loads all TC data from disk (or embedded bytes on WASM):

- `Tc::load(dir)` — native file-system loader
- `Tc::load_with(reader)` — generic loader (used by WASM with `include_dir`)
- `Weapon`, `NObjectType`, `SObjectType` — serde TOML structs
- `TcData` — top-level config (physics constants, texts, hacks, type lists)
- `parse_large_sprites`, `parse_small_sprites` — TGA decoder
- `parse_wav_f32` — WAV decoder (8-bit unsigned → f32 PCM)

### `liero-sim`

Deterministic, headless game simulation. No OS or render dependencies:

- `Game::new(tc, num_worms, seed)` — creates a game
- `Game::step(inputs)` — advances one frame
- `Game::checksum()` — FNV-1a hash of all mutable game state (for desync detection)
- Pure fixed-point arithmetic; reproducible across platforms

### `liero-render`

wgpu-based palette renderer:

- Single 8-bit frame buffer (320×200 pixels, palette indices)
- One GPU texture upload per frame
- Palette LUT via wgpu shader
- CRT scanline overlay (optional)
- Split-screen compositing (two 158×158 viewports + HUD strip)

### `liero-audio`

cpal-based audio engine:

- Loads `SoundSamples` from `Tc::sounds`
- Plays samples via `play_sound(idx)` (fire-and-forget)
- Falls back to a silent engine if audio init fails

### `liero-net`

Lockstep rollback session:

- `RollbackSession::host(addr, local_player)` — bind and wait for peer
- `RollbackSession::connect(local, remote, local_player)` — connect to host
- `RollbackSession::step(game, local_input)` — exchange inputs and advance simulation
- TC hash mismatch detection at frame ≤ 5
- `tc_hash_of(tc)` — FNV-1a TC fingerprint

### `liero-mod`

Lua 5.4 modding layer (via `mlua` with vendored Lua):

- `apply_mod(tc, lua_source)` — run a Lua script against a mutable `Tc`
- Exposes `weapons`, `nobjects`, `sobjects` as Lua tables
- Writes changes back after the script completes

### `liero-desktop`

Native platform entry point:

- winit event loop, wgpu surface, gilrs gamepad
- Menu UI rendered via `liero-render`
- Loads TC → applies `mod.lua` → starts game
- CLI: `openliero [TC_PATH] [--host-udp PORT] [--connect-udp ADDR:PORT]`

### `liero-web`

WASM entry point (compiled with `wasm-pack`):

- Embeds `TC/openliero` at compile time via `include_dir!`
- `Tc::load_with` with an embedded-dir reader — no async I/O needed
- `wasm_bindgen_futures::spawn_local` for async wgpu init
- `winit::platform::web::EventLoopExtWebSys::spawn_app` for the event loop

### `tools/liero-relay`

Standalone relay server (no workspace dependencies):

- TCP listener on `:7777`, WebSocket listener on `:7778`
- `HOST\n` → generates 6-letter room code, responds `ROOM:XXXXXX\n`
- `JOIN:XXXXXX\n` → pairs with host, bidirectional byte forwarding

### `tools/tc-validator`

TC directory validation CLI:

- Loads TC via `liero-data`
- Cross-validates all string references (splinter_type, create_on_exp, etc.)
- Reports counts and any broken references to stderr

### `tools/replay-diff`

Determinism checker:

- Runs two replay files through `liero-sim` in parallel
- Reports the first frame where checksums diverge
