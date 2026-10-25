#!/usr/bin/env bash
# Render a frame scene to target/ssr/<slug>.html (PLAN D57).
#   scripts/ssr.sh "Projects / empty / 1440"
set -euo pipefail

FRAME="${1:?usage: scripts/ssr.sh \"<frame name>\"}"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

SLUG="$(printf '%s' "$FRAME" | tr '[:upper:]' '[:lower:]' | sed 's| / |-|g; s| |-|g; s|/|-|g')"
mkdir -p target/ssr
cargo run -q -p berth-app --example ssr -- "$FRAME" > "target/ssr/$SLUG.html"
echo "wrote target/ssr/$SLUG.html"
