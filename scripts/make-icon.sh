#!/usr/bin/env bash
# Build Berth's app icon set from one 1024x1024 PNG (P10-T1, PLAN §12 item 1).
#   scripts/make-icon.sh <1024.png>
# Writes crates/berth-app/assets/icon/{berth.icns,icon.png,32x32.png,128x128.png,
# 128x128@2x.png,256x256.png,512x512.png}. macOS only (sips, iconutil). The PNG must be
# square and 1024 px wide, with the transparent margin Apple's grid asks for already in it.
# Until the chosen artwork lands, run it on icon-placeholder-1024.png.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: scripts/make-icon.sh <1024.png>" >&2
  exit 2
fi
src="$1"
[ -f "$src" ] || { echo "make-icon: $src not found" >&2; exit 1; }

w="$(sips -g pixelWidth "$src" | awk '/pixelWidth/ {print $2}')"
h="$(sips -g pixelHeight "$src" | awk '/pixelHeight/ {print $2}')"
if [ "$w" != 1024 ] || [ "$h" != 1024 ]; then
  echo "make-icon: $src is ${w}x${h}, need 1024x1024" >&2
  exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/crates/berth-app/assets/icon"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
set="$work/berth.iconset"
mkdir -p "$set" "$out"

# Apple's iconset names: size and @2x pairs for 16, 32, 128, 256, 512.
resize() { sips -s format png -z "$1" "$1" "$src" --out "$2" >/dev/null; }
resize 16   "$set/icon_16x16.png"
resize 32   "$set/icon_16x16@2x.png"
resize 32   "$set/icon_32x32.png"
resize 64   "$set/icon_32x32@2x.png"
resize 128  "$set/icon_128x128.png"
resize 256  "$set/icon_128x128@2x.png"
resize 256  "$set/icon_256x256.png"
resize 512  "$set/icon_256x256@2x.png"
resize 512  "$set/icon_512x512.png"
cp "$src"   "$set/icon_512x512@2x.png"

iconutil --convert icns --output "$out/berth.icns" "$set"

# The PNG sizes crates/berth-app/Dioxus.toml lists.
resize 32   "$out/32x32.png"
resize 128  "$out/128x128.png"
resize 256  "$out/128x128@2x.png"
resize 256  "$out/256x256.png"
resize 512  "$out/512x512.png"
cp "$src"   "$out/icon.png"

echo "make-icon: wrote $out/berth.icns and 6 PNGs from $src"
