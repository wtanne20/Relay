#!/usr/bin/env bash
# Install Relay's "hands" role on a Proot Linux phone.
#
#   ./install.sh            # install into ~/.relay-host and show the start line
#   RELAY_ROOT=/home/you/app ./install.sh
#
# What it does:
#   - downloads the aarch64 relay binary (from the current GitHub release)
#   - puts it in ~/.relay-host/
#   - prints the one line that starts the host, and the address + token to
#     paste into the phone app (Settings -> Devices)
#
# The model, agent loop and chat history all stay on your PC. This only
# exposes the device's folder and shell to the agent.

set -euo pipefail

VERSION="$(curl -fsSL https://api.github.com/repos/wtanne20/Relay/releases/latest 2>/dev/null \
  | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n1 || true)"
VERSION="${VERSION:-latest}"
URL="https://github.com/wtanne20/Relay/releases/download/v${VERSION}/relay-host-aarch64-linux.zip"
DEST="${RELY_HOME:-$HOME/.relay-host}"
ROOT="${RELY_ROOT:-$HOME}"

echo "Installing Relay host (v${VERSION}) into ${DEST}"
mkdir -p "$DEST"

# Download. Prefer curl; fall back to wget.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
if command -v curl >/dev/null 2>&1; then
  curl -fL --retry 3 -o "$tmp/relay-host.zip" "$URL"
elif command -v wget >/dev/null 2>&1; then
  wget -O "$tmp/relay-host.zip" "$URL"
else
  echo "Need curl or wget to download." >&2
  exit 1
fi

# Unzip. Proot images usually have unzip; otherwise use python.
if command -v unzip >/dev/null 2>&1; then
  (cd "$tmp" && unzip -q relay-host.zip)
elif command -v python3 >/dev/null 2>&1; then
  (cd "$tmp" && python3 -m zipfile -e relay-host.zip .)
else
  echo "Need unzip or python3 to extract the zip." >&2
  exit 1
fi

install -m 0755 "$tmp/relay" "$DEST/relay"
install -m 0755 "$tmp/start" "$DEST/start" 2>/dev/null || \
  printf '#!/bin/sh\nexec "$(dirname "$0")/relay" host --bind 0.0.0.0:8766 --root "%s" "$@"\n' "$ROOT" > "$DEST/start"
chmod +x "$DEST/start"
# The PWA launcher (opens the brain's web UI in a browser).
if [ -f "$tmp/pwa" ]; then
  install -m 0755 "$tmp/pwa" "$DEST/pwa"
fi

echo
echo "Installed. To start the host, run:"
echo
echo "  $DEST/start"
echo
echo "It will print the address and token. On your PC, open the Relay app:"
echo "  Settings -> Devices -> Pair a device, and paste that address + token."
echo
echo "To change the folder the agent can touch, set RELY_ROOT before starting:"
echo "  RELY_ROOT=$ROOT $DEST/start"
echo
echo "To get the Relay UI on this box (a full-screen app in your browser):"
echo "  $DEST/pwa http://your-pc:8765"
echo "  (open it, then use 'Add to Home Screen' / 'Install app')"
