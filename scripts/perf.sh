#!/usr/bin/env bash
# Timing checks (AD26, PLAN section 9 row 6), in two stages.
#   scripts/perf.sh               stage 1 only
#   scripts/perf.sh --app         stage 1, then stage 2
#   scripts/perf.sh --app-only    stage 2 only
# Stage 1: the tests named perf_*, which scripts/check.sh leaves out (the default nextest
# profile filters them and they are #[ignore]). Release build, one test at a time, output kept
# so the numbers print. Extra arguments go to nextest.
# Stage 2 (scripts/perf_app.py): the release app against the real engine with 50 containers
# (scripts/fx-up.sh and scripts/fx-up.sh --perf; berthfx-broken down). Prints cold start to a
# populated Projects table (< 600 ms), idle CPU with the window closed for 60 s (< 0.5 %)
# and memory with the window open (< 150 MB). Needs Accessibility for osascript. Exit 1 when
# a number misses its target.
set -euo pipefail

cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

STAGE1=1
STAGE2=0
EXTRA=()
for arg in "$@"; do
  case "$arg" in
    --app) STAGE2=1 ;;
    --app-only) STAGE1=0; STAGE2=1 ;;
    *) EXTRA+=("$arg") ;;
  esac
done

if [ "$STAGE1" = 1 ]; then
  cargo nextest run --workspace --release --profile perf --run-ignored only --no-capture ${EXTRA[@]+"${EXTRA[@]}"}
fi

if [ "$STAGE2" = 1 ]; then
  python3 -I scripts/perf_app.py
fi
