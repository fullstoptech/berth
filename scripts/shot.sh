#!/usr/bin/env bash
# Capture the Berth window to target/shots/<name>.png (+ a <name>-1x.png copy). PLAN D57.
#   scripts/shot.sh <name> [--scene "<frame>"]
# Launches the app with BERTH_MOCK=1, the scene (when given) and a temp BERTH_DATA_DIR,
# waits for the window, reads its bounds with osascript and captures with screencapture -R.
# BERTH_SHOT_ATTACH=1 captures the already running Berth instead (no build or launch).
# BERTH_THEME and other BERTH_* vars pass through. BERTH_SHOT_WAIT: seconds to let the
# window paint (default 8).
set -euo pipefail

NAME="${1:?usage: scripts/shot.sh <name> [--scene \"<frame>\"]}"
shift
SCENE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --scene) SCENE="${2:?--scene needs a frame name}"; shift 2 ;;
    *) echo "shot.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done

cd "$(dirname "${BASH_SOURCE[0]}")/.."
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

mkdir -p target/shots
OUT="$PWD/target/shots/$NAME.png"
WAIT="${BERTH_SHOT_WAIT:-8}"
ATTACH="${BERTH_SHOT_ATTACH:-0}"

APP_PID=""
DATA_DIR=""
cleanup() {
  if [ -n "$APP_PID" ]; then
    kill "$APP_PID" 2>/dev/null || true
  fi
  if [ -n "$DATA_DIR" ]; then
    rm -rf "$DATA_DIR"
  fi
}
trap cleanup EXIT

if [ "$ATTACH" = 1 ]; then
  WIN_PID="$(pgrep -nx berth || true)"
  [ -n "$WIN_PID" ] || { echo "shot.sh: BERTH_SHOT_ATTACH=1 but no berth process is running" >&2; exit 1; }
else
  cargo build -p berth-app
  APP="${CARGO_TARGET_DIR:-$PWD/target}/debug/berth"
  [ -x "$APP" ] || APP="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/debug/berth"
  DATA_DIR="$(mktemp -d)"
  BERTH_MOCK=1 BERTH_SCENE="$SCENE" BERTH_DATA_DIR="$DATA_DIR" "$APP" &
  APP_PID=$!
  WIN_PID=$APP_PID
  sleep "$WAIT"
fi

osascript -e 'tell application "System Events" to set frontmost of (first process whose unix id is '"$WIN_PID"') to true' >/dev/null 2>&1 || true
sleep 0.5
# Window id and bounds via CoreGraphics (scripts/winbounds.swift): osascript System
# Events needs Accessibility, which not every terminal holding a session has.
HELPER="target/winbounds"
if [ ! -x "$HELPER" ] || [ scripts/winbounds.swift -nt "$HELPER" ]; then
  swiftc -O -o "$HELPER" scripts/winbounds.swift
fi
INFO=""
for _ in $(seq 1 20); do
  INFO="$("$HELPER" "$WIN_PID" || true)"
  [ -n "$INFO" ] && break
  sleep 0.5
done
if [ -n "$INFO" ]; then
  IFS=, read -r WIN_ID X Y W H <<<"$INFO"
  screencapture -x -o -l "$WIN_ID" "$OUT"
else
  BOUNDS="$(osascript -e 'tell application "System Events" to tell (first process whose unix id is '"$WIN_PID"') to get {position, size} of window 1' | tr -d ' ')"
  IFS=, read -r X Y W H <<<"$BOUNDS"
  screencapture -x -R "$X,$Y,$W,$H" "$OUT"
fi
sips -Z "$W" "$OUT" --out "${OUT%.png}-1x.png" >/dev/null 2>&1 || true
echo "wrote $OUT"
