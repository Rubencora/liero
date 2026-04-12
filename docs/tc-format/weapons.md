# Weapons (`weapons/*.cfg`)

Each weapon is a TOML file in `weapons/`. The filename (without `.cfg`) must match the entry in `tc.cfg [types] weapons`.

## All fields

All integer fields default to `0`, all booleans to `false`, all optional strings to `nil`, unless noted.

### Flags (boolean)

| Field | Description |
|-------|-------------|
| `affectByWorm` | Projectile is deflected by worm collision |
| `shadow` | Draws a drop shadow under the projectile |
| `laserSight` | Draws a laser targeting line |
| `playReloadSound` | Play the reload sound on firing |
| `wormExplode` | Explodes on worm contact |
| `explGround` | Explodes on terrain contact |
| `wormCollide` | Collides with worms (without exploding) |
| `collideWithObjects` | Collides with other projectiles |
| `affectByExplosions` | Can be pushed by nearby explosions |
| `loopAnim` | Loop the sprite animation |
| `chainExplosion` | Triggers a chain reaction with nearby weapons |

### Integer params

| Field | Default | Description |
|-------|---------|-------------|
| `speed` | `100` | Initial speed (fixed-point units) |
| `addSpeed` | `0` | Speed added per step |
| `multSpeed` | `100` | Speed multiplier per step (100 = no change) |
| `distribution` | `0` | Spread / random angle deviation |
| `parts` | `1` | Number of projectiles fired at once |
| `recoil` | `0` | Recoil applied to the firing worm |
| `delay` | `0` | Frames between shots |
| `loadingTime` | `0` | Reload time in frames |
| `ammo` | `0` | Max ammo (0 = unlimited) |
| `gravity` | `0` | Gravity applied to projectile each frame |
| `bounce` | `0` | Bounce coefficient on terrain hit |
| `timeToExplo` | `0` | Frames before auto-explosion (0 = never) |
| `timeToExploV` | `0` | Random variance added to `timeToExplo` |
| `hitDamage` | `0` | Damage on worm hit |
| `bloodOnHit` | `0` | Blood particles on hit |
| `detectDistance` | `0` | Homing detection radius |
| `blowAway` | `0` | Force applied to worm on hit |
| `launchSound` | `0` | Sound index to play on fire |
| `loopSound` | `0` | Sound index to loop while in flight |
| `exploSound` | `-1` | Sound index on explosion (-1 = none) |
| `dirtEffect` | `-1` | Terrain modification (-1=none, 0=dig, 1=larpa, 2=deposit) |
| `leaveShells` | `0` | Number of shell particles to emit |
| `leaveShellDelay` | `1` | Delay between shell emissions |
| `fireCone` | `0` | Fire cone angle (0 = straight) |
| `startFrame` | `0` | First sprite frame (small.tga index) |
| `numFrames` | `0` | Number of animation frames |
| `shotType` | `0` | Shot behaviour (0=normal, 2=steerable, 4=laser, 5=homing) |
| `colorBullets` | `0` | Palette index for pixel-coloured projectile |
| `splinterAmount` | `0` | Fragments on explosion |
| `splinterColour` | `0` | Palette index for splinter particles |
| `splinterScatter` | `0` | Random scatter of splinter angle |
| `objTrailDelay` | `0` | Frames between object trail emissions |
| `partTrailType` | `0` | Particle trail type index |
| `partTrailDelay` | `0` | Frames between particle trail emissions |

### Optional string references

| Field | Description |
|-------|-------------|
| `splinterType` | Name of the nobject type to emit as splinters |
| `objTrailType` | Name of the nobject type to emit as object trail |
| `partTrailObj` | Name of the nobject type to emit as particle trail |
| `createOnExp` | Name of the sobject type to create on explosion |

### Extension fields (Sprint 7+)

| Field | Default | Description |
|-------|---------|-------------|
| `homingStrength` | `0` | Homing turn rate |
| `attractRadius` | `0` | Attract-force detection radius |
| `attractForce` | `0` | Force magnitude for attract-type homing |
| `chainLightningJumps` | `0` | Number of chain-lightning hops |
| `onExpireTeleport` | `false` | Teleport worm to projectile on expiry |
| `dirtDeposit` | `false` | Deposit terrain on contact |
| `pierceDirt` | `false` | Pass through terrain without exploding |

## Example

```toml
# weapons/Bazooka.cfg
name       = "Bazooka"
speed      = 600
gravity    = 30
explGround = true
wormExplode = true
hitDamage  = 50
timeToExplo = 0
createOnExp = "BigExplosion"
splinterType = "Rocket"
splinterAmount = 8
startFrame = 10
numFrames  = 1
```
