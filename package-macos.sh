#!/usr/bin/env sh
# Builds Faust and packages it as a native macOS .app bundle.
#
#   ./package-macos.sh            build, bundle, and install into /Applications
#   ./package-macos.sh build      build and bundle only (leaves Faust.app in ./dist)
#   ./package-macos.sh uninstall  remove /Applications/Faust.app
#
# Needs only cargo plus tools shipped with macOS (iconutil, sips, qlmanage).
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
APP_NAME="Faust"
BIN_NAME="faust"
BUNDLE_ID="com.nak.faust"
VERSION=$(awk -F'"' '/^version *=/{print $2; exit}' "$HERE/Cargo.toml")
DIST="$HERE/dist"
APP="$DIST/$APP_NAME.app"
DEST="/Applications/$APP_NAME.app"

if [ "${1:-install}" = "uninstall" ]; then
    rm -rf "$DEST"
    echo "$APP_NAME: removed $DEST"
    exit 0
fi

command -v cargo >/dev/null 2>&1 || {
    echo "$APP_NAME: cargo not found. Install Rust first: https://rustup.rs" >&2
    exit 1
}

echo "$APP_NAME: building release binary..."
cargo build --release --manifest-path "$HERE/Cargo.toml"

echo "$APP_NAME: assembling bundle (v$VERSION)..."
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$HERE/target/release/$BIN_NAME" "$APP/Contents/MacOS/$BIN_NAME"

# --- icon: SVG -> 1024px PNG -> .iconset (all sizes) -> .icns ---------------
ICONSET=$(mktemp -d)/$APP_NAME.iconset
mkdir -p "$ICONSET"
PNGDIR=$(mktemp -d)
qlmanage -t -s 1024 -o "$PNGDIR" "$HERE/assets/$BIN_NAME.svg" >/dev/null 2>&1
BASE="$PNGDIR/$BIN_NAME.svg.png"
gen() { sips -z "$2" "$2" "$BASE" --out "$ICONSET/$1" >/dev/null; }
gen icon_16x16.png       16
gen icon_16x16@2x.png    32
gen icon_32x32.png       32
gen icon_32x32@2x.png    64
gen icon_128x128.png     128
gen icon_128x128@2x.png  256
gen icon_256x256.png     256
gen icon_256x256@2x.png  512
gen icon_512x512.png     512
gen icon_512x512@2x.png  1024
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/$BIN_NAME.icns"
rm -rf "$ICONSET" "$PNGDIR"

# --- Info.plist -------------------------------------------------------------
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>     <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>      <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>         <string>$VERSION</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundleExecutable</key>      <string>$BIN_NAME</string>
    <key>CFBundleIconFile</key>        <string>$BIN_NAME</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>LSMinimumSystemVersion</key>  <string>10.13</string>
    <key>NSHighResolutionCapable</key> <true/>
    <key>LSApplicationCategoryType</key> <string>public.app-category.productivity</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP/Contents/PkgInfo"

# An ad-hoc signature keeps Gatekeeper from killing an unsigned bundle
# outright on Apple Silicon. Not a substitute for a Developer ID cert.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo "$APP_NAME: bundle ready at $APP"

if [ "${1:-install}" = "build" ]; then
    exit 0
fi

echo "$APP_NAME: installing to ${DEST}..."
rm -rf "$DEST"
cp -R "$APP" "$DEST"
echo "$APP_NAME: installed. Launch it from /Applications or Spotlight."
echo
echo "First launch: macOS may warn it's from an unidentified developer."
echo "Right-click Faust.app -> Open, then confirm -- or run:"
echo "    xattr -dr com.apple.quarantine \"$DEST\""
