# TC Format — Overview

A **Tournament Config (TC)** is a directory containing all the data that defines a Liero ruleset:
weapons, particles, explosion effects, physics constants, sprites, and sounds.

## Directory layout

```
TC/my-tc/
├── tc.cfg                # Top-level config: constants, texts, hacks, type lists
├── weapons/
│   ├── Bazooka.cfg       # One file per weapon
│   ├── Shotgun.cfg
│   └── …
├── nobjects/
│   ├── Rocket.cfg        # Nobject (non-owner projectile) types
│   └── …
├── sobjects/
│   ├── BigExplosion.cfg  # Sobject (screen-space effect) types
│   └── …
├── sprites/
│   ├── large.tga         # 16×16 sprite sheet (110 large frames + worm frames 16-36)
│   └── small.tga         # 7×7 sprite sheet (130 small frames)
├── sounds/
│   ├── AMMO.WAV
│   └── …
└── mod.lua               # (Optional) Lua mod script, applied at game start
```

## File formats

| File | Format | Notes |
|------|--------|-------|
| `*.cfg` | TOML | camelCase keys, `#[serde(default)]` for all fields |
| `sprites/large.tga` | TGA (paletted, 8-bit) | 16px wide, height = 16 × num_frames |
| `sprites/small.tga` | TGA (paletted, 8-bit) | 7px wide, height = 7 × num_frames |
| `sounds/*.WAV` | PCM WAV | 22050 Hz, mono, 8-bit unsigned |
| `mod.lua` | Lua 5.4 | Optional; see [Modding](../modding/overview.md) |

## Type lists

`tc.cfg` contains a `[types]` section that names all the weapons, nobjects, sobjects, and sounds.
OpenLiero loads each named item from its subdirectory:

```toml
[types]
weapons  = ["Bazooka", "Shotgun", "Larpa"]
nobjects = ["Rocket", "Splinter"]
sobjects = ["BigExplosion", "SmallExplosion"]
sounds   = ["AMMO", "EXPLO1", "EXPLO2"]
```

## Validation

Use the `tc-validator` tool to check a TC before distributing it:

```bash
cargo run -p tc-validator -- TC/my-tc
```

See [tc-validator](../tools/tc-validator.md) for details.
