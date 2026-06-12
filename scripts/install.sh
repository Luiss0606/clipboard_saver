#!/usr/bin/env bash
# Installs (or updates) Clipboard Saver into /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/Luiss0606/clipboard_saver/main/scripts/install.sh | bash
#
# Downloading with curl avoids the com.apple.quarantine attribute, so
# Gatekeeper never blocks the ad-hoc-signed app.
set -euo pipefail

REPO="Luiss0606/clipboard_saver"
ASSET="ClipboardSaver.app.zip"
APP_NAME="Clipboard Saver.app"
DEST="/Applications/$APP_NAME"

echo "==> Looking up latest release of $REPO"
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep -o "\"browser_download_url\": *\"[^\"]*$ASSET\"" \
  | sed -E 's/.*"(https[^"]*)"/\1/' \
  | head -1)

if [ -z "$URL" ]; then
  echo "error: could not find $ASSET in the latest release" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Downloading $URL"
curl -fsSL -o "$TMP/$ASSET" "$URL"

echo "==> Installing into /Applications"
pkill -x clipboard_saver 2>/dev/null || true
ditto -x -k "$TMP/$ASSET" "$TMP/extracted"
rm -rf "$DEST"
mv "$TMP/extracted/$APP_NAME" "$DEST"

# Defensive: clears quarantine left behind by older browser-based installs.
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

echo "==> Launching"
open "$DEST"

echo "Done. Open the 📋 menu and enable \"Iniciar con el sistema\" if you want it at boot."
