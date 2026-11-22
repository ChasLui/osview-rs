#!/usr/bin/env bash
# Builds dist/osview-rs.app around the GUI binary. A bare executable launched
# from a shell cannot reliably become the frontmost app on macOS (focus APIs
# report success while the previous app keeps receiving keys), so packaging a
# real .app is what makes keyboard/mouse verification and `open -a` work.
#
#   scripts/bundle-gui.sh [cargo-target]
#
# The optional target (e.g. aarch64-apple-darwin) is for release builds; the
# bundle always keeps the host layout, so it must match the host architecture.

set -euo pipefail
cd "$(dirname "$0")/.."

meta="$(cargo metadata --format-version 1 --no-deps)"
version="$(printf '%s' "$meta" | jq -r '.packages[0].version')"
target_dir="$(printf '%s' "$meta" | jq -r '.target_directory')"
app="dist/osview-rs.app"

build=(cargo build --release --locked --features gui --bin osview-gui)
bin_dir="$target_dir/release"
if [[ $# -gt 0 ]]; then
    build+=(--target "$1")
    bin_dir="$target_dir/$1/release"
fi
"${build[@]}"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$bin_dir/osview-gui" "$app/Contents/MacOS/osview-gui"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleIdentifier</key><string>dev.chaslui.osview-rs</string>
    <key>CFBundleName</key><string>osview-rs</string>
    <key>CFBundleDisplayName</key><string>osview-rs</string>
    <key>CFBundleExecutable</key><string>osview-gui</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

plutil -lint "$app/Contents/Info.plist" >/dev/null
printf 'built %s (v%s)\n' "$app" "$version"
