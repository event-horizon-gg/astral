#!/bin/sh
set -eu
cd "$(dirname "$0")"
version=$(awk '
  /^\[package\]$/ { in_package = 1; next }
  /^\[/ { in_package = 0 }
  in_package && /^version = "[0-9][0-9.]*"$/ { gsub(/"/, "", $3); print $3; exit }
' Cargo.toml)
case "$(uname -m)" in
  arm64) arch=arm64 ;;
  x86_64) arch=x64 ;;
  *) exit 1 ;;
esac
[ "$(uname -s)" = Darwin ] && [ -n "$version" ]
cargo build --locked --release
binary="${CARGO_TARGET_DIR:-target}/release/astral-native"
output="$(pwd)/../release"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app="$stage/Astral Native.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$output"
cp "$binary" "$app/Contents/MacOS/astral-native"
cp ../assets/icon.png "$app/Contents/Resources/Astral.png"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>astral-native</string>
<key>CFBundleIdentifier</key><string>com.joaoseixas.astral.native</string>
<key>CFBundleName</key><string>Astral Native</string>
<key>CFBundleIconFile</key><string>Astral.png</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
/usr/bin/codesign --force --sign - "$app"
/usr/bin/codesign --verify --deep --strict "$app"
archive="Astral-Native-$version-mac-$arch.zip"
(cd "$stage" && /usr/bin/zip -q -r "$archive" 'Astral Native.app')
cp "$stage/$archive" "$output/$archive"
printf '%s\n' "$output/$archive"
