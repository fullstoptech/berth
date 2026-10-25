#!/usr/bin/env bash
# Fails when a secret-shaped value appears in shipped code (crates/, plugins/, web-src/).
# Docs and design files name these patterns to describe the rule; they are out of scope,
# as are fixtures and scripts (PLAN P0-T3). Masked values (value starting with a bullet) pass.
set -eu

cd "$(dirname "$0")/.."

dirs=""
for d in crates plugins web-src; do
  if [ -d "$d" ]; then
    dirs="$dirs $d"
  fi
done

fail=0
scan() {
  pattern="$1"
  label="$2"
  # shellcheck disable=SC2086
  if grep -rnIE "$pattern" $dirs; then
    echo "secret-scan: $label" >&2
    fail=1
  fi
}

scan 'berthfx-dev' 'fixture password outside fixtures/ and scripts/'
scan 'BERTH_BRIDGE_TOKEN=[A-Za-z0-9]' 'bridge token value in code'
scan 'PASSWORD=[A-Za-z0-9]' 'password value in code'

exit "$fail"
