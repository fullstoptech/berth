#!/usr/bin/env bash
# Canonical gate (PLAN section 8): format, lint, test, token freshness, scans.
set -euo pipefail

cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
python3 -I scripts/gen-tokens.py design/tokens.json | diff -q - crates/berth-app/assets/css/tokens.css >/dev/null
scripts/secret-scan.sh
scripts/css-lint.sh
