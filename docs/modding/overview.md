# Modding Overview

OpenLiero supports Lua 5.4 mods that can modify TC data at game start.
No recompilation is needed — just drop a `mod.lua` file into a TC directory.

## How it works

1. The game loads the TC from disk as usual.
2. If `<TC_DIR>/mod.lua` exists, it is executed with the `liero-mod` Lua engine.
3. The script can freely read and modify weapons, nobjects, and sobjects via the three globals: `weapons`, `nobjects`, `sobjects`.
4. After the script returns, all changes are written back into the in-memory TC.
5. The game starts with the patched TC. **Original TC files on disk are never modified.**

## Globals

| Global | Type | Contents |
|--------|------|----------|
| `weapons` | array (1-indexed) | One table per weapon in `tc.cfg [types] weapons` |
| `nobjects` | array (1-indexed) | One table per nobject type |
| `sobjects` | array (1-indexed) | One table per sobject type |

Each table contains all the fields documented in the TC format pages, using **snake_case** names (e.g., `hit_damage`, `worm_explode`).

Boolean fields are represented as integers: `0` = false, `1` = true.

## Error handling

If the script raises a Lua error or has a syntax error, the error is printed to stderr and the game starts with the unmodified TC. The game never crashes due to a mod error.

```
[mod] applied mod.lua from "TC/openliero/mod.lua"
[mod] mod.lua error: [string "mod.lua"]:3: attempt to perform arithmetic on a nil value
```

## Safety

The Lua VM is initialised with the standard Lua 5.4 library. No OS, file, or network access is exposed beyond what Lua's standard library provides by default.

!!! warning "Standard Lua I/O"
    The standard Lua `io`, `os`, and `package` libraries are available in the sandbox.
    If you want to distribute mods and need stricter isolation, start the game from a
    restricted user account or remove those libraries from `liero-mod/src/lib.rs`.

## Next steps

- [Lua API reference](lua-api.md)
- [Example mods](examples.md)
