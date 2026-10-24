#!/usr/bin/env bash
# Start Berth fixture Compose projects (PLAN 8.1). Touches only resources
# labelled dev.berth.fixture=1.
#   scripts/fx-up.sh [--broken] [--registry]
set -euo pipefail

cd "$(dirname "$0")/.."

BROKEN=0
REGISTRY=0
for arg in "$@"; do
  case "$arg" in
    --broken) BROKEN=1 ;;
    --registry) REGISTRY=1 ;;
    *) echo "fx-up.sh: unknown argument $arg" >&2; exit 2 ;;
  esac
done

compose() {
  dir="$1"
  name="$2"
  shift 2
  docker compose --project-directory "$dir" -f "$dir/compose.yaml" -p "$name" "$@"
}

compose fixtures/compose/berthfx berthfx up -d --wait

if [ "$REGISTRY" = 1 ]; then
  compose fixtures/compose/berthfx-registry berthfx-registry up -d --wait
fi

if [ "$BROKEN" = 1 ]; then
  # The broken set is expected to fail in assorted ways; bring up what comes up.
  compose fixtures/compose/berthfx-broken berthfx-broken up -d || true
fi
