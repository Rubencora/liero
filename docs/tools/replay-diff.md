# replay-diff

`replay-diff` checks two replay files for determinism divergence by running them
both through `liero-sim` and comparing checksums frame by frame.

## Usage

```bash
cargo run --manifest-path openliero-rs/Cargo.toml -p replay-diff -- <REPLAY_A> <REPLAY_B>
```

## Output

If the replays are identical (fully deterministic):

```
Replays match: 3600 frames identical.
```

If they diverge:

```
Divergence at frame 142:
  A checksum: 0xdeadbeef12345678
  B checksum: 0xcafebabe87654321
```

## Use cases

- **Verifying a sim refactor** didn't break determinism: record a replay before and after, then diff.
- **Debugging desync** in online matches: both players export their replay; `replay-diff` shows exactly when the sim diverged.

## Replay format

Replays are a flat binary file:

```
[0..3]   magic:    b"LREP"
[4..7]   version:  u32 le
[8..11]  seed:     u32 le
[12..15] tc_hash:  (unused, 0-padded to 8 bytes)
[20..]   frames:   repeated u32 le input values, one per worm per frame
```

The frame count is derived from the file length.
