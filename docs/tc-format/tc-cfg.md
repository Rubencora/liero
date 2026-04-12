# tc.cfg

`tc.cfg` is the top-level TOML config for a Tournament Config. It contains:

- `[types]` — lists of weapon/nobject/sobject/sound names
- `[constants]` — physics constants (worm physics, aiming, bonuses, etc.)
- `[texts]` — UI strings (kill messages, HUD labels)
- `[hacks]` — boolean feature flags (fall damage, air jump, etc.)

## `[types]`

```toml
[types]
sounds   = ["AMMO", "EXPLO1", "EXPLO2"]
weapons  = ["Bazooka", "Shotgun", "Larpa"]
nobjects = ["Rocket", "Splinter"]
sobjects = ["BigExplosion", "SmallExplosion"]
```

The order of entries determines the index used in `launch_sound`, `explo_sound`, etc.

## `[constants]` — physics (selection)

| Key | Description |
|-----|-------------|
| `WormGravity` | Per-frame gravity applied to worms |
| `WalkVelLeft` / `WalkVelRight` | Walking acceleration |
| `MaxVelLeft` / `MaxVelRight` | Max walking speed |
| `JumpForce` | Upward impulse on jump |
| `WormFricMult` / `WormFricDiv` | Ground friction (multiplier / divisor) |
| `NRInitialLength` | Initial ninja rope length |
| `BonusGravity` | Gravity on dropped bonuses |
| `materials` | 256-byte array mapping palette indices to material types |

## `[hacks]`

| Key | Default | Description |
|-----|---------|-------------|
| `FallDamage` | `false` | Enable fall damage |
| `BonusReloadOnly` | `false` | Bonuses only grant ammo, not health |
| `WormFloat` | `false` | Worms float on terrain |
| `RemExp` | `false` | Remove expiry explosions |
| `AirJump` | `false` | Allow jumping mid-air |
| `MultiJump` | `false` | Allow infinite air jumps |

## `[texts]`

| Key | Description |
|-----|-------------|
| `KilledMsg` | Kill message format (e.g., `"%s killed %s"`) |
| `CommittedSuicideMsg` | Suicide message |
| `PressFireToBegin` | Menu prompt |
| `Kills` | HUD label |
| `Lives` | HUD label |
