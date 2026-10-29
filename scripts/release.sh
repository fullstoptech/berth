#!/usr/bin/env bash
# Build Berth's release .app and Berth-<version>-arm64.dmg (P10-T2, PLAN §12 items 3 and 4).
#   scripts/release.sh
# macOS (Apple silicon) only. Always a release build. Output goes to dist/:
#   dist/Berth.app, dist/Berth-<version>-arm64.dmg (the DMG path is the last line printed).
# Re-runnable: it removes its own output first.
#
# Signing is opt-in through the environment and never happens otherwise:
#   BERTH_SIGN_IDENTITY   e.g. "Developer ID Application: Name (TEAMID)". Signs berth-coolify,
#                         the app (hardened runtime, timestamp, packaging/macos/Berth.entitlements)
#                         and the DMG.
#   BERTH_NOTARY_PROFILE  notarytool keychain profile. Needs BERTH_SIGN_IDENTITY. Notarizes and
#                         staples the app and the DMG.
# Without BERTH_SIGN_IDENTITY the app and DMG are unsigned (right-click Open on first launch).
# Other knobs: BERTH_DMG_FORMAT (default UDZO; ULFO is smaller), BERTH_DMG_MAX_BYTES (default
# 31457280 = 30 MiB).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

die() { echo "release: $*" >&2; exit 1; }
say() { echo "release: $*"; }

[ "$(uname -s)" = Darwin ] || die "macOS only"
[ "$(uname -m)" = arm64 ] || die "Apple silicon only (the DMG is named arm64)"

if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
need=(cargo rustc dx hdiutil ditto)
sign_id="${BERTH_SIGN_IDENTITY:-}"
notary="${BERTH_NOTARY_PROFILE:-}"
[ -z "$notary" ] || [ -n "$sign_id" ] || die "BERTH_NOTARY_PROFILE is set but BERTH_SIGN_IDENTITY is not; notarizing needs a signed app"
[ -z "$sign_id" ] || need+=(codesign)
[ -z "$notary" ] || need+=(xcrun)
for tool in "${need[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' not found on PATH"
done
entitlements="$root/packaging/macos/Berth.entitlements"
[ -z "$sign_id" ] || [ -f "$entitlements" ] || die "$entitlements not found"

pkgid="$(cargo pkgid -p berth-app)"
version="${pkgid##*[#@]}"
[ -n "$version" ] || die "could not read the version from 'cargo pkgid -p berth-app' ($pkgid)"
say "version $version"

dmg_name="Berth-$version-arm64.dmg"
max_bytes="${BERTH_DMG_MAX_BYTES:-31457280}"
dist="$root/dist"
stage="$dist/.dmg-stage"
app="$dist/Berth.app"
dmg="$dist/$dmg_name"

# dx strips the binary with the toolchain's rust-objcopy, which cannot find libLLVM.dylib unless
# the toolchain's lib directory is on the dylib path.
sysroot="$(rustc --print sysroot)"
dx_env=()
if [ -d "$sysroot/lib" ]; then
  dx_env=(env "DYLD_LIBRARY_PATH=$sysroot/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}")
fi

target_dir="${CARGO_TARGET_DIR:-$root/target}"
bundle_root="$target_dir/dx/berth/bundle/macos"

say "cleaning dist/ and the previous dx bundle"
rm -rf "$stage" "$app" "$dmg" "$bundle_root"
mkdir -p "$dist"

say "dx bundle --release (this takes a while)"
"${dx_env[@]}" dx bundle --release --platform macos -p berth-app

built="$(find "$bundle_root" -maxdepth 2 -name Berth.app -type d | head -n 1)"
[ -n "$built" ] || die "dx produced no Berth.app under $bundle_root"
ditto "$built" "$app"
[ -x "$app/Contents/MacOS/berth" ] || die "$app/Contents/MacOS/berth is missing or not executable"

say "building berth-coolify (release)"
cargo build --release -p berth-coolify
coolify="$target_dir/release/berth-coolify"
[ -x "$coolify" ] || die "$coolify was not built"
# The plugin binary lives in Contents/MacOS, where code signing wants executables (AD48, AD70).
cp "$coolify" "$app/Contents/MacOS/berth-coolify"
# Its manifest ships as a sealed resource; the host resolves the executable in Contents/MacOS and
# never runs anything from Resources (AD70). Copied before signing so the app signature seals it.
manifest="$root/plugins/coolify/berth-plugin.json"
[ -f "$manifest" ] || die "$manifest is missing; the app would ship without discovering the Coolify plugin (AD70)"
mkdir -p "$app/Contents/Resources/plugins/coolify"
cp "$manifest" "$app/Contents/Resources/plugins/coolify/berth-plugin.json"

sign_one() { # sign_one <path> [extra codesign args...]
  local path="$1"; shift
  codesign --force --options runtime --timestamp --sign "$sign_id" "$@" "$path"
}

notarize() { # notarize <zip-or-dmg>
  xcrun notarytool submit "$1" --keychain-profile "$notary" --wait
}

if [ -n "$sign_id" ]; then
  say "signing with '$sign_id' (inside out)"
  sign_one "$app/Contents/MacOS/berth-coolify"
  sign_one "$app/Contents/MacOS/berth"
  sign_one "$app" --entitlements "$entitlements"
  codesign --verify --deep --strict --verbose=2 "$app"
  if [ -n "$notary" ]; then
    say "notarizing the app"
    zip="$dist/.Berth-notarize.zip"
    rm -f "$zip"
    ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip"
    rm -f "$zip"
    xcrun stapler staple "$app"
  fi
else
  say "BERTH_SIGN_IDENTITY is not set: the app is UNSIGNED"
fi

app_kb="$(du -sk "$app" | awk '{print $1}')"
say "Berth.app is $((app_kb / 1024)) MiB on disk"

say "building $dmg_name"
mkdir -p "$stage"
ditto "$app" "$stage/Berth.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname Berth -srcfolder "$stage" -ov -format "${BERTH_DMG_FORMAT:-UDZO}" \
  -fs HFS+ -imagekey zlib-level=9 "$dmg" >/dev/null
rm -rf "$stage"

if [ -n "$sign_id" ]; then
  say "signing the DMG"
  codesign --force --timestamp --sign "$sign_id" "$dmg"
  if [ -n "$notary" ]; then
    say "notarizing the DMG"
    notarize "$dmg"
    xcrun stapler staple "$dmg"
  fi
fi

size="$(stat -f %z "$dmg")"
say "$dmg_name is $size bytes (limit $max_bytes)"
if [ "$size" -gt "$max_bytes" ]; then
  die "$dmg_name is $size bytes, over the $max_bytes limit (30 MiB, PLAN §12 item 4). Check the release profile (lto, strip) and what grew in Berth.app."
fi
hdiutil verify "$dmg" >/dev/null || die "hdiutil verify failed for $dmg"

if [ -z "$sign_id" ]; then
  say "UNSIGNED build. Set BERTH_SIGN_IDENTITY (and BERTH_NOTARY_PROFILE) to sign and notarize."
fi
echo "$dmg"
