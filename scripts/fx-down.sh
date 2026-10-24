#!/usr/bin/env bash
# Remove Berth fixture projects and any labelled leftovers (PLAN 8.1).
# Only resources labelled dev.berth.fixture=1 are ever touched.
set -eu

cd "$(dirname "$0")/.."

for spec in "fixtures/compose/berthfx berthfx" "fixtures/compose/berthfx-broken berthfx-broken" "fixtures/compose/berthfx-registry berthfx-registry"; do
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
