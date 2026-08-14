#!/usr/bin/env bash
set -euo pipefail

HEDGENTS_ANCHOR_BIN="${HEDGENTS_ANCHOR_CLI:-anchor}"
HEDGENTS_EXPECTED_ANCHOR="anchor-cli 0.31.1"
HEDGENTS_ACTUAL_ANCHOR="$("$HEDGENTS_ANCHOR_BIN" --version)"

if [[ "$HEDGENTS_ACTUAL_ANCHOR" != "$HEDGENTS_EXPECTED_ANCHOR" ]]; then
  echo "Expected $HEDGENTS_EXPECTED_ANCHOR, found $HEDGENTS_ACTUAL_ANCHOR." >&2
  echo "Select Anchor 0.31.1 or set HEDGENTS_ANCHOR_CLI to its exact binary path." >&2
  exit 1
fi

exec "$HEDGENTS_ANCHOR_BIN" build "$@"
