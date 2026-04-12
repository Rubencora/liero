# SObjects (`sobjects/*.cfg`)

SObjects are screen-space explosion effects — animated sprites that play at a fixed position.
Each is a TOML file in `sobjects/`.

## Fields

| Field | Default | Description |
|-------|---------|-------------|
| `startFrame` | `0` | First frame index in `large.tga` |
| `numFrames` | `1` | Number of animation frames |
| `animDelay` | `1` | Frames per animation step |
| `startSound` | `-1` | Sound index to play on creation (-1 = none) |
| `numSounds` | `0` | Number of sounds to randomly pick from (starting at `startSound`) |
| `detectRange` | `0` | Damage radius |
| `damage` | `0` | Damage applied to worms in range |
| `blowAway` | `0` | Knockback force |
| `shake` | `0` | Screen shake magnitude |
| `flash` | `0` | Screen flash brightness |
| `dirtEffect` | `-1` | Terrain modification (-1=none, 0=dig) |
| `shadow` | `false` | Draw drop shadow |

## Example

```toml
# sobjects/BigExplosion.cfg
startFrame   = 0
numFrames    = 12
animDelay    = 2
startSound   = 3
numSounds    = 2
detectRange  = 40
damage       = 80
blowAway     = 2000
shake        = 30
flash        = 10
dirtEffect   = 0
```
