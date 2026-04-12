# Network Protocol

OpenLiero uses **lockstep TCP** (not UDP rollback) for online multiplayer.
Both players must run at the same tick rate; the game waits for the peer's
input before advancing the simulation.

## Transport

| Aspect | Value |
|--------|-------|
| Protocol | TCP (direct) or WebSocket (via relay) |
| Default port | `7777` (direct), `7778` (WebSocket relay) |
| Framing | Fixed-size binary packets (no length prefix) |

## Packet format

Each direction sends one packet per simulation frame:

```
[0..3]  frame:    u32 le   — simulation frame number
[4..7]  input:    u32 le   — local player input bitmask
[8..15] checksum: u64 le   — FNV-1a game state hash
```

Total: **16 bytes** per frame per direction.

## Desync detection

After exchanging packets, each player checks:

```
received.checksum == local_checksum(received.frame)
```

If the checksums differ at frame ≤ 5, it is classified as a **TC mismatch** —
both players are likely using different tournament configs.  The error message
includes the local TC hash so players can compare:

```
TC mismatch at frame 2 — ensure both players load the same TC
(local hash: 3f7a2c1b4e5d6a8f)
```

For divergence after frame 5, a general **game state desync** is reported.

## Input bitmask

| Bit | Action |
|-----|--------|
| `0` | Move left |
| `1` | Move right |
| `2` | Aim up |
| `3` | Aim down |
| `4` | Fire |
| `5` | Change weapon |
| `6` | Jump |
| `7` | (reserved) |

Bits 8-31 carry the same pattern for players 3-4 (local split-screen worms
in an online session are packed into the same u32).

## TC hash

The TC hash (`liero_net::tc_hash_of`) is a FNV-1a hash over:

- `palette` (768 bytes)
- weapon count + weapon names
- nobject count
- sobject count

This is a lightweight sanity check, not a cryptographic guarantee.
