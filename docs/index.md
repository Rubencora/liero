# OpenLiero

**OpenLiero** is a faithful Rust reimplementation of the classic 1998 worm-shooter *Liero* by Joosa Riekkinen, featuring:

- **Native desktop build** — wgpu renderer, gilrs gamepad support, local 2-player split screen
- **Online multiplayer** — TCP/WebSocket lockstep networking with a relay server and 6-letter room codes
- **WASM build** — play in any modern browser via WebAssembly
- **Lua modding** — drop a `mod.lua` in any TC folder to tweak weapons, nobjects, and explosion effects
- **TC validation** — built-in `tc-validator` CLI to catch broken Tournament Configs before a match

---

## Quick start

```bash
# Clone and build
git clone https://github.com/openliero/openliero.git
cd openliero
cargo run --manifest-path openliero-rs/Cargo.toml -p liero-desktop -- TC/openliero
```

See [Building from source](getting-started/building.md) for full instructions.

---

## Project layout

| Path | Description |
|------|-------------|
| `openliero-rs/crates/liero-data` | TC loader — weapons, nobjects, sobjects, sprites |
| `openliero-rs/crates/liero-sim` | Deterministic game simulation (headless) |
| `openliero-rs/crates/liero-render` | wgpu-based palette renderer |
| `openliero-rs/crates/liero-audio` | cpal audio engine |
| `openliero-rs/crates/liero-net` | Rollback-lockstep networking |
| `openliero-rs/crates/liero-mod` | Lua 5.4 modding layer via mlua |
| `openliero-rs/crates/liero-desktop` | Native entry point (winit + gilrs) |
| `openliero-rs/crates/liero-web` | WASM entry point (wasm-bindgen) |
| `openliero-rs/tools/liero-relay` | Relay server (TCP + WebSocket) |
| `openliero-rs/tools/tc-validator` | Tournament Config validator CLI |
| `openliero-rs/tools/replay-diff` | Determinism checker for replays |
| `TC/openliero` | Default OpenLiero tournament config |
| `web/` | HTML + WASM package for the browser build |
