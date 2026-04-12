# NObjects (`nobjects/*.cfg`)

NObjects (non-owner projectiles) are particles emitted by weapons — splinters, rockets, blood, etc.
Each is a TOML file in `nobjects/`.

## Fields

| Field | Default | Description |
|-------|---------|-------------|
| `gravity` | `0` | Per-frame gravity |
| `speed` | `0` | Initial speed magnitude |
| `speedV` | `0` | Vertical speed component |
| `distribution` | `0` | Random angle spread |
| `blowAway` | `0` | Knockback on worm impact |
| `bounce` | `0` | Terrain bounce coefficient |
| `hitDamage` | `0` | Damage on worm contact |
| `bloodOnHit` | `0` | Blood particles on impact |
| `detectDistance` | `0` | Proximity trigger range |
| `startFrame` | `0` | First sprite frame (small.tga index); ≤ 0 → pixel-coloured |
| `numFrames` | `0` | Number of animation frames |
| `colorBullets` | `0` | Palette index if rendered as a pixel |
| `dirtEffect` | `-1` | Terrain modification (-1=none, 0=dig, 1=larpa) |
| `splinterAmount` | `0` | Fragments emitted on expiry |
| `splinterColour` | `0` | Palette index for splinter pixels |
| `bloodTrailDelay` | `0` | Frames between blood trail drops |
| `leaveObjDelay` | `0` | Frames between leave-object emissions |
| `timeToExplo` | `0` | Auto-expiry timer (0 = never) |
| `timeToExploV` | `0` | Expiry timer variance |
| `wormExplode` | `false` | Explode on worm contact |
| `explGround` | `false` | Explode on terrain |
| `wormDestroy` | `false` | Destroy on worm contact (no explosion) |
| `drawOnMap` | `false` | Draw into terrain (permanent pixel) |
| `affectByExplosions` | `false` | Pushed by nearby explosions |
| `bloodTrail` | `false` | Emit blood trail while moving |
| `createOnExp` | `nil` | Sobject type to create on expiry |
| `splinterType` | `nil` | Nobject type for splinters |
| `leaveObj` | `nil` | Nobject type to periodically emit |
