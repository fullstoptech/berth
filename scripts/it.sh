#!/usr/bin/env bash
# Integration tests against the live engine: bring fixtures up, then run the
# ignored tests (they gate on BERTH_IT=1). PLAN section 8.
#   scripts/it.sh [-p crate] [filter]
set -euo pipefail

cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

PKG_ARGS=(--workspace)
FILTER=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) PKG_ARGS=(-p "${2:?it.sh: -p needs a crate name}"); shift 2 ;;
    *) FILTER="$1"; shift ;;
  esac
done

export BERTH_IT=1
scripts/fx-up.sh

NEXTEST_ARGS=(run "${PKG_ARGS[@]}" --run-ignored all)
if [ -n "$FILTER" ]; then
  NEXTEST_ARGS+=("$FILTER")
fi
# bash 3.2: expanding an empty array under set -u errors; NEXTEST_ARGS is never
# empty here, but keep the safe form (Ferry Mac session lesson).
cargo nextest "${NEXTEST_ARGS[@]+"${NEXTEST_ARGS[@]}"}"
