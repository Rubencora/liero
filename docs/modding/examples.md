# Mod Examples

## Chaos mode — everything faster

```lua
-- chaos.lua
-- Triple all weapon speeds and halve reload times.

for _, w in ipairs(weapons) do
    w.speed      = math.floor(w.speed * 3)
    w.add_speed  = math.floor(w.add_speed * 3)
    w.delay      = math.max(1, math.floor(w.delay / 2))
    w.loading_time = math.max(0, math.floor(w.loading_time / 2))
end
```

## Big boom — amplify all explosion damage

```lua
-- bigboom.lua
-- Multiply all sobject explosion damage by 3, and
-- double hit damage on all weapons.

for _, s in ipairs(sobjects) do
    s.damage   = s.damage * 3
    s.blow_away = math.floor(s.blow_away * 2)
end

for _, w in ipairs(weapons) do
    w.hit_damage = w.hit_damage * 2
end
```

## Gravity flip — weapons float upward

```lua
-- lowgrav.lua
-- Negate gravity for all weapons and nobjects.

for _, w in ipairs(weapons) do
    w.gravity = -math.abs(w.gravity)
end

for _, n in ipairs(nobjects) do
    n.gravity = -math.abs(n.gravity)
end
```

## Sniper — find by name and buff

```lua
-- sniper.lua
-- Make the sniper rifle (if present) fire faster and do more damage.

local function find(name)
    for _, w in ipairs(weapons) do
        if w.name == name then return w end
    end
end

local sniper = find("Sniper")
if sniper then
    sniper.speed      = sniper.speed * 4
    sniper.hit_damage = sniper.hit_damage * 3
    sniper.delay      = 1
end
```

## Debug — print all TC data

```lua
-- debug.lua
-- Useful when building a new TC to verify values.

io.write("=== WEAPONS ===\n")
for i, w in ipairs(weapons) do
    io.write(string.format("[%02d] %-22s  spd=%-5d dmg=%-4d parts=%d\n",
        i, w.name, w.speed, w.hit_damage, w.parts))
end

io.write("\n=== NOBJECTS ===\n")
for i, n in ipairs(nobjects) do
    io.write(string.format("[%02d] gravity=%-5d speed=%-5d dmg=%d\n",
        i, n.gravity, n.speed, n.hit_damage))
end

io.write("\n=== SOBJECTS ===\n")
for i, s in ipairs(sobjects) do
    io.write(string.format("[%02d] damage=%-4d blow_away=%-4d shake=%d\n",
        i, s.damage, s.blow_away, s.shake))
end
```
