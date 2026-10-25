#!/usr/bin/env bash
# Timing checks (D26): the tests named perf_*, which scripts/check.sh leaves
# out (the default nextest profile filters them and they are #[ignore]).
# Release build, one test at a time, output kept so the numbers print.
set -euo pipefail

cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

exec cargo nextest run --workspace --release --profile perf --run-ignored only --no-capture "$@"
