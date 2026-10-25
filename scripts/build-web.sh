#!/usr/bin/env bash
# Rebuild the sealed web bundles (PLAN D9, ADR 0004) into
# crates/berth-app/assets/web/{terminal,editor}.{js,css}. Needs Node 24 and pnpm 12
# and the npm registry once; sessions that do not touch web-src/ never run this.
# pnpm 12 refuses packages younger than 24 h and needs `pnpm approve-builds esbuild`
# once on a fresh checkout (web-src/pnpm-workspace.yaml records the approval).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../web-src"

if ! command -v node >/dev/null 2>&1 && command -v fnm >/dev/null 2>&1; then
  eval "$(fnm env --shell bash)"
  fnm use 24.20.0 >/dev/null
fi

node_version="$(node --version)"
case "$node_version" in
  v24.*) ;;
  *) echo "build-web: Node 24 is required (found $node_version)" >&2; exit 1 ;;
esac

pnpm install --frozen-lockfile
node build.mjs
