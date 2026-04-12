# Sprites

OpenLiero uses two TGA sprite sheets: `sprites/large.tga` (16×16) and `sprites/small.tga` (7×7).
Both use the same palette (extracted from `large.tga`).

## large.tga

| Attribute | Value |
|-----------|-------|
| Format | TGA, paletted 8-bit |
| Frame size | 16 × 16 pixels |
| Width | 16 px (fixed) |
| Height | 16 × N px (one frame stacked per 16 rows) |
| Frame count | First 110 frames = sobject/bonus sprites; frames 16–36 = worm source frames |

### Frame layout (standard openliero TC)

| Frames | Content |
|--------|---------|
| 0–15 | Misc large sprites (bonuses, objects) |
| 16–36 | Worm source sprites (21 frames × 5 angles + walk phases) |
| 37–109 | Sobject animation frames |

### Worm sprites

Worm sprites are generated at load time from frames 16–36 of `large.tga`:

- 21 source frames × 2 directions × 4 worm colour slots = **168 pre-baked sprites**
- Direction 0 (left) = horizontally mirrored
- Colour slots: palette indices 30–34 are remapped to `30 + 9 * slot`

## small.tga

| Attribute | Value |
|-----------|-------|
| Format | TGA, paletted 8-bit |
| Frame size | 7 × 7 pixels |
| Width | 7 px (fixed) |
| Height | 7 × N px |
| Frame count | 130 frames (standard) |

Small sprites are used for:
- Weapon projectile sprites (if `startFrame ≥ 0` in the weapon config)
- NObject sprites (if `startFrame > 0`)

## Palette

The 256-colour palette is stored in the TGA colour map of `large.tga` (RGB 8-8-8, 768 bytes).
The same palette is applied to both sprite sheets.

Colour index 0 is treated as **transparent** in all sprite blitting.
