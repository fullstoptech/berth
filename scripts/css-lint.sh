#!/usr/bin/env bash
# Fails on any hex colour or rgb() in component CSS (tokens.css is exempt). PLAN D4, 7.7.
set -eu

cd "$(dirname "$0")/.."
dir="crates/berth-app/assets/css"

files=""
for f in "$dir"/*.css; do
  [ -f "$f" ] || continue
  [ "$f" = "$dir/tokens.css" ] && continue
  files="$files $f"
done

if [ -z "$files" ]; then
  exit 0
fi

# shellcheck disable=SC2086
if grep -En '#[0-9a-fA-F]{3,8}([[:space:];,)]|$)|\brgb\(' $files; then
  echo "css-lint: hex colours and rgb() belong in tokens.css only (use var(--color-*))" >&2
  exit 1
fi
