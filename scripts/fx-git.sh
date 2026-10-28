#!/usr/bin/env bash
# Build fixtures/git/berthfx-app.git (bare, two commits) from the committed
# source tree in fixtures/git/berthfx-app/ (PLAN 8.1, P8-T5). The deploy-
# from-Git fixture: Berth clones it, a later commit shows up as incoming.
# Idempotent: the bare repo is rebuilt from scratch on every run, with fixed
# author/committer identity and dates so the commit hashes are stable.
# Touches only fixtures/git/ and a temp dir; never the engine.
set -euo pipefail

cd "$(dirname "$0")/.."

SRC="fixtures/git/berthfx-app"
BARE="fixtures/git/berthfx-app.git"

if ! command -v git >/dev/null 2>&1; then
  echo "fx-git.sh: git not found on PATH" >&2
  exit 1
fi
if [ ! -f "$SRC/compose.yaml" ]; then
  echo "fx-git.sh: $SRC/compose.yaml missing" >&2
  exit 1
fi

export GIT_AUTHOR_NAME="Berth Fixtures"
export GIT_AUTHOR_EMAIL="fixtures@berth.invalid"
export GIT_COMMITTER_NAME="Berth Fixtures"
export GIT_COMMITTER_EMAIL="fixtures@berth.invalid"
export GIT_AUTHOR_DATE="2026-01-01T00:00:00Z"
export GIT_COMMITTER_DATE="2026-01-01T00:00:00Z"
GIT=(git -c commit.gpgsign=false)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

"${GIT[@]}" init -q -b main "$WORK/repo"
cp -R "$SRC/." "$WORK/repo/"
"${GIT[@]}" -C "$WORK/repo" add -A
"${GIT[@]}" -C "$WORK/repo" commit -q -m "berthfx-app: initial commit"

printf -- '- Fixture build note (commit two, added by scripts/fx-git.sh).\n' >>"$WORK/repo/CHANGELOG.md"
GIT_AUTHOR_DATE="2026-01-02T00:00:00Z" GIT_COMMITTER_DATE="2026-01-02T00:00:00Z" \
  "${GIT[@]}" -C "$WORK/repo" commit -q -am "berthfx-app: note the fixture build"

rm -rf "$BARE"
"${GIT[@]}" clone -q --bare "$WORK/repo" "$BARE"

echo "fx-git: $BARE @ $(git -C "$BARE" rev-parse --short HEAD) ($(git -C "$BARE" rev-list --count HEAD) commits, branch main)"
