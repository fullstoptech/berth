#!/usr/bin/env bash
# Remove Berth fixture projects and any labelled leftovers (PLAN 8.1).
# Only resources labelled dev.berth.fixture=1 are ever touched.
#   scripts/fx-down.sh          everything labelled dev.berth.fixture=1
#   scripts/fx-down.sh --perf   only berthfx-perf (labelled dev.berth.fixture.set=perf)
set -eu

cd "$(dirname "$0")/.."

if [ "${1:-}" = "--perf" ]; then
  docker compose --project-directory fixtures/compose/berthfx-perf -f fixtures/compose/berthfx-perf/compose.yaml -p berthfx-perf down -v --remove-orphans >/dev/null 2>&1 || true
  docker ps -aq --filter label=dev.berth.fixture.set=perf | while read -r id; do
    [ -n "$id" ] && docker rm -f "$id" >/dev/null
  done
  docker network ls -q --filter label=dev.berth.fixture.set=perf | while read -r id; do
    [ -n "$id" ] && docker network rm "$id" >/dev/null 2>&1 || true
  done
  exit 0
fi

for spec in "fixtures/compose/berthfx berthfx" "fixtures/compose/berthfx-broken berthfx-broken" "fixtures/compose/berthfx-registry berthfx-registry" "fixtures/compose/berthfx-perf berthfx-perf"; do
  # shellcheck disable=SC2086
  set -- $spec
  docker compose --project-directory "$1" -f "$1/compose.yaml" -p "$2" down -v --remove-orphans >/dev/null 2>&1 || true
done

# Label sweep for anything the compose files no longer know about.
docker ps -aq --filter label=dev.berth.fixture=1 | while read -r id; do
  [ -n "$id" ] && docker rm -f "$id" >/dev/null
done
docker volume ls -q --filter label=dev.berth.fixture=1 | while read -r name; do
  [ -n "$name" ] && docker volume rm "$name" >/dev/null
done
docker network ls -q --filter label=dev.berth.fixture=1 | while read -r id; do
  [ -n "$id" ] && docker network rm "$id" >/dev/null 2>&1 || true
done
