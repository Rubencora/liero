# tc-validator

`tc-validator` validates a Tournament Config directory and reports any issues.

## Usage

```bash
cargo run --manifest-path openliero-rs/Cargo.toml -p tc-validator -- <TC_PATH>

# Example
cargo run --manifest-path openliero-rs/Cargo.toml -p tc-validator -- TC/openliero
```

## What it checks

- **Loadability** — the TC directory can be fully loaded by `liero-data`
- **Cross-references** — all string references resolve to real types:
    - `weapon.splinter_type` → must name a valid nobject
    - `weapon.obj_trail_type` → must name a valid nobject
    - `weapon.part_trail_obj` → must name a valid nobject
    - `weapon.create_on_exp` → must name a valid sobject
    - `nobject.create_on_exp` → must name a valid sobject
    - `nobject.splinter_type` → must name a valid nobject
    - `nobject.leave_obj` → must name a valid nobject
- **Counts** — prints a summary of weapons / nobjects / sobjects / sounds found

## Output

On success:

```
TC loaded: TC/openliero
  weapons:  40
  nobjects: 28
  sobjects: 12
  sounds:   30
All references OK.
```

On error:

```
TC loaded: TC/mymod
  weapons:  5
  nobjects: 3
  sobjects: 2
  sounds:   10
ERROR: weapon "Bazooka" create_on_exp = "HugeExplosion" — not found in sobjects
ERROR: weapon "Rocket" splinter_type = "Shard" — not found in nobjects
2 error(s) found.
```

Exit code is `0` on success, `1` on any error.

## Use in CI

```yaml
# .github/workflows/validate-tc.yml
- name: Validate TC
  run: |
    cargo run -p tc-validator -- TC/openliero
    cargo run -p tc-validator -- TC/mymod
```
