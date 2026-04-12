# Running OpenLiero

## Basic usage

```bash
openliero [TC_PATH]
```

`TC_PATH` defaults to `TC/openliero` relative to the working directory.  
The last used TC is saved to `~/.config/openliero/config.toml`.

### Examples

```bash
# Default TC
cargo run --manifest-path openliero-rs/Cargo.toml -p liero-desktop

# Custom TC
cargo run --manifest-path openliero-rs/Cargo.toml -p liero-desktop -- TC/mymod

# Release build
./openliero-rs/target/release/openliero TC/openliero
```

---

## Online multiplayer

### Host a game

```bash
openliero TC/openliero --host-udp 7777
```

Share your IP and port with the other player.

### Join a game

```bash
openliero TC/openliero --connect-udp 1.2.3.4:7777
```

### Via relay server

If both players are behind NAT, use the relay:

1. Start the relay: `cargo run -p liero-relay` (or use the public instance)
2. One player enters the 6-letter room code shown by the relay
3. The other player connects with the same code

See [Relay Server](../net/relay.md) for details.

---

## Mod loading

If a `mod.lua` file exists in the TC directory, it is executed automatically at game start.
The original TC files are never modified. See [Modding overview](../modding/overview.md).

---

## Configuration file

`~/.config/openliero/config.toml` (created on first run):

```toml
last_tc   = "TC/openliero"
last_mode = 0          # 0=Last Man Standing … 5=Juggernaut
host_port = "7777"
join_addr = "127.0.0.1:7777"
scanlines = false      # CRT scanline overlay (toggle with F2 in-game)
```
