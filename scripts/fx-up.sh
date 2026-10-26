#!/usr/bin/env bash
# Start Berth fixture Compose projects (PLAN 8.1). Touches only resources
# labelled dev.berth.fixture=1.
#   scripts/fx-up.sh [--broken | --broken-only] [--registry] [--perf]
# --broken-only brings up just berthfx-broken (the live problems tests call it and
# take the project down again themselves). --perf brings up just berthfx-perf (46 idle
# busybox containers; with the four berthfx ones that is the 50 of scripts/perf.sh --app) and
# leaves berthfx alone. scripts/fx-down.sh --perf takes it down alone.
set -euo pipefail

cd "$(dirname "$0")/.."

BROKEN=0
NORMAL=1
REGISTRY=0
PERF=0
for arg in "$@"; do
  case "$arg" in
    --broken) BROKEN=1 ;;
    --broken-only) BROKEN=1; NORMAL=0 ;;
    --registry) REGISTRY=1 ;;
    --perf) PERF=1; NORMAL=0 ;;
    *) echo "fx-up.sh: unknown argument $arg" >&2; exit 2 ;;
  esac
done

compose() {
  dir="$1"
  name="$2"
  shift 2
  docker compose --project-directory "$dir" -f "$dir/compose.yaml" -p "$name" "$@"
}

if [ "$NORMAL" = 1 ]; then
  compose fixtures/compose/berthfx berthfx up -d --wait
fi

if [ "$REGISTRY" = 1 ]; then
  compose fixtures/compose/berthfx-registry berthfx-registry up -d --wait
fi

if [ "$PERF" = 1 ]; then
  compose fixtures/compose/berthfx-perf berthfx-perf up -d --wait
fi

if [ "$BROKEN" = 1 ]; then
  # The broken set fails in assorted ways, and compose aborts a whole `up` when one
  # service cannot be made (the image that does not exist). So bring each service up
  # on its own and keep going; the failures are the point of the set.
  broken=fixtures/compose/berthfx-broken
  # Start from nothing: a re-run would otherwise restart the port clash's container, and
  # Docker lets that second start through without the port, so the clash would vanish.
  compose "$broken" berthfx-broken down -v --remove-orphans >/dev/null 2>&1 || true
  for svc in $(compose "$broken" berthfx-broken config --services); do
    if ! compose "$broken" berthfx-broken up -d --no-deps "$svc"; then
      echo "fx-up.sh: berthfx-broken/$svc did not come up (expected for a broken fixture)" >&2
    fi
  done
fi
