#!/usr/bin/env bash
# Builds Clipboard Saver.app and packages it into ClipboardSaver.dmg.
# Requires cargo-bundle: cargo install cargo-bundle
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="Clipboard Saver"
DMG_NAME="ClipboardSaver.dmg"
BUNDLE_DIR="target/release/bundle/osx"
APP_PATH="$BUNDLE_DIR/$APP_NAME.app"

echo "==> Building .app bundle (release)"
cargo bundle --release

echo "==> Marking app as menu-bar-only (LSUIElement)"
/usr/libexec/PlistBuddy -c "Add :LSUIElement bool true" "$APP_PATH/Contents/Info.plist" 2>/dev/null \
  || /usr/libexec/PlistBuddy -c "Set :LSUIElement true" "$APP_PATH/Contents/Info.plist"

echo "==> Ad-hoc code signing"
codesign --force --deep -s - "$APP_PATH"

echo "==> Creating $DMG_NAME"
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP_PATH" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
rm -f "$DMG_NAME"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGING" -ov -format UDZO "$DMG_NAME"

echo "==> Done: $(pwd)/$DMG_NAME"
echo "    Open the dmg and drag '$APP_NAME.app' into /Applications."
