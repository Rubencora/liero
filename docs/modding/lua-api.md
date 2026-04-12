# Lua API Reference

## Globals

### `weapons` — weapon array

```lua
-- weapons[i] is a table with all weapon fields (1-indexed)
for i, w in ipairs(weapons) do
    print(w.name, w.speed)
end
```

Fields mirror [Weapons](../tc-format/weapons.md) — snake_case, booleans as 0/1 integers:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Weapon name (read-only for display; changing it has no sim effect) |
| `speed` | integer | Initial projectile speed |
| `add_speed` | integer | Speed delta per frame |
| `mult_speed` | integer | Speed multiplier per frame (100 = no change) |
| `distribution` | integer | Random angle spread |
| `parts` | integer | Projectiles per shot |
| `gravity` | integer | Per-frame gravity |
| `bounce` | integer | Terrain bounce coefficient |
| `delay` | integer | Frames between shots |
| `loading_time` | integer | Reload duration |
| `ammo` | integer | Max ammo (0 = unlimited) |
| `recoil` | integer | Recoil force on firing worm |
| `hit_damage` | integer | Damage on worm impact |
| `blood_on_hit` | integer | Blood particles on impact |
| `time_to_explo` | integer | Auto-explosion timer (0 = none) |
| `time_to_explo_v` | integer | Timer variance |
| `detect_distance` | integer | Homing detection range |
| `blow_away` | integer | Knockback on impact |
| `worm_explode` | 0 or 1 | Explode on worm contact |
| `expl_ground` | 0 or 1 | Explode on terrain |
| `worm_collide` | 0 or 1 | Collide with worms |
| `affect_by_explosions` | 0 or 1 | Pushed by explosions |
| `shot_type` | integer | 0=normal, 2=steerable, 4=laser, 5=homing |
| `dirt_effect` | integer | -1=none, 0=dig, 1=larpa, 2=deposit |
| `splinter_type` | string or nil | Nobject type for splinters |
| `create_on_exp` | string or nil | Sobject type on explosion |
| `homing_strength` | integer | Homing turn rate |
| _(all other fields from weapons.md)_ | | |

### `nobjects` — nobject type array

```lua
-- nobjects[i] is a table with all NObjectType fields
nobjects[1].gravity = nobjects[1].gravity * 2
```

Fields mirror [NObjects](../tc-format/nobjects.md).

### `sobjects` — sobject type array

```lua
-- sobjects[i] is a table with all SObjectType fields
for _, s in ipairs(sobjects) do
    s.damage = math.floor(s.damage * 1.5)
end
```

Fields mirror [SObjects](../tc-format/sobjects.md).

---

## Standard Lua libraries available

The full Lua 5.4 standard library is available: `math`, `string`, `table`, `io`, `os`, `package`, etc.

---

## Tips

### Find a weapon by name

```lua
local function find_weapon(name)
    for _, w in ipairs(weapons) do
        if w.name == name then return w end
    end
    return nil
end

local bazooka = find_weapon("Bazooka")
if bazooka then
    bazooka.hit_damage = bazooka.hit_damage * 2
end
```

### Scale all weapon speeds

```lua
for _, w in ipairs(weapons) do
    w.speed     = math.floor(w.speed * 1.5)
    w.add_speed = math.floor(w.add_speed * 1.5)
end
```

### Print a summary for debugging

```lua
print("=== Weapons ===")
for i, w in ipairs(weapons) do
    print(string.format("  [%d] %-20s speed=%d parts=%d dmg=%d",
        i, w.name, w.speed, w.parts, w.hit_damage))
end
```
