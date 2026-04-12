# Sounds (`sounds/*.WAV`)

Sound effects are 8-bit unsigned PCM WAV files stored in `sounds/`.

## Format

| Attribute | Value |
|-----------|-------|
| Format | PCM WAV |
| Sample rate | 22050 Hz |
| Bit depth | 8-bit unsigned |
| Channels | Mono |

The file name (uppercase, without `.WAV`) must match the entry in `tc.cfg [types] sounds`.

## Loading

OpenLiero decodes each WAV at load time with `liero-data::parse_wav_f32`:
- 8-bit unsigned samples are normalised to `f32` in the range −1.0 to +1.0
- `value_f32 = (sample_u8 as f32 - 128.0) / 128.0`

## Referencing sounds

Sound indices are 0-based and match the order in `tc.cfg [types] sounds`:

```toml
[types]
sounds = ["AMMO", "EXPLO1", "EXPLO2", "EXPLO3"]
#          0       1         2          3
```

In weapon configs:
```toml
launchSound = 0   # plays AMMO.WAV on fire
exploSound  = 1   # plays EXPLO1.WAV on explosion
```

A value of `-1` means no sound.

## Fallback

If a sound file is missing or corrupt, the entry is silently replaced with an empty
sample buffer (silence). The game never fails to start due to a missing sound.
