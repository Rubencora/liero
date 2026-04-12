# Building from source

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust toolchain | ≥ 1.75 | Install via [rustup](https://rustup.rs/) |
| `cargo` | ships with Rust | — |
| C compiler | any | Required by mlua (Lua 5.4 vendored) |

Optional for WASM:

| Tool | Version | Notes |
|------|---------|-------|
| `wasm-pack` | ≥ 0.13 | `cargo install wasm-pack` |
| Python 3 | any | Only for the `make serve-web` dev server |

---

## Native desktop (macOS / Linux / Windows)

```bash
git clone https://github.com/openliero/openliero.git
cd openliero
cargo build --manifest-path openliero-rs/Cargo.toml -p liero-desktop
```

The resulting binary is at `openliero-rs/target/debug/openliero`.

For an optimised release build:

```bash
cargo build --manifest-path openliero-rs/Cargo.toml -p liero-desktop --release
```

---

## WASM (browser)

```bash
make web          # builds WASM package into web/pkg/
make serve-web    # starts a local HTTP server on port 8080
```

Then open `http://localhost:8080` in your browser.

!!! note "WASM embeds the TC at compile time"
    The WASM build uses `include_dir!` to embed `TC/openliero` directly into
    the binary. Rebuilding after TC changes is required.

---

## Relay server

```bash
cargo run --manifest-path openliero-rs/Cargo.toml -p liero-relay
# Listening on TCP :7777 and WebSocket :7778
```

See [Relay Server](../net/relay.md) for deployment notes.

---

## Tools

```bash
# Validate a TC directory
cargo run --manifest-path openliero-rs/Cargo.toml -p tc-validator -- TC/openliero

# Check two replays for determinism divergence
cargo run --manifest-path openliero-rs/Cargo.toml -p replay-diff -- a.rep b.rep
```
