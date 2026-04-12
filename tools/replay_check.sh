#!/usr/bin/env bash
# CI harness: run a replay and compare CRCs against a golden file.
# Usage: ./tools/replay_check.sh <replay.lrp> <golden_crcs.csv>
set -euo pipefail

GAME="./build/openliero.app/Contents/MacOS/openliero"
REPLAY="$1"
GOLDEN="$2"
TMP_CRCS=$(mktemp /tmp/replay_crcs_XXXXXX.csv)

echo "[replay_check] Running replay: $REPLAY"
"$GAME" --headless --replay "$REPLAY" --dump-crcs "$TMP_CRCS"

echo "[replay_check] Comparing CRCs..."
if diff -q "$GOLDEN" "$TMP_CRCS" > /dev/null 2>&1; then
    echo "[replay_check] PASS: CRCs match golden file"
    rm -f "$TMP_CRCS"
    exit 0
else
    echo "[replay_check] FAIL: CRC mismatch!"
    diff "$GOLDEN" "$TMP_CRCS" | head -20
    rm -f "$TMP_CRCS"
    exit 1
fi
