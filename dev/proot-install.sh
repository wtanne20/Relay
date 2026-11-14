#!/usr/bin/env bash
# Link a Linux device to your Relay brain from a terminal (a phone running
# Proot, or a Linux PC without a desktop).
#
#   ./install.sh
#
# What it does:
#   - downloads the relay binary for this machine's architecture (x86_64 or
#     aarch64) from the current GitHub release into ~/.relay-host/
#   - asks for your brain's Tailscale address and the 6-digit code the brain
#     shows (Settings -> Computers -> Link a computer), and pairs
#   - prints the one line that keeps this device linked
#
# The model, agent loop and chat history all stay on your brain. This only
# lets the brain's agent work in this device's folder. To chat from this
# device too, install the Relay desktop app (.deb) from the same release.

set -euo pipefail

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)  ASSET="relay-host-x86_64-linux.zip" ;;
  aarch64|arm64) ASSET="relay-host-aarch64-linux.zip" ;;
  *) echo "Unsupported architecture: $ARCH (need x86_64 or aarch64)" >&2; exit 1 ;;
esac

VERSION="$(curl -fsSL https://api.github.com/repos/wtanne20/Relay/releases/latest 2>/dev/null \
  | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n1 || true)"
VERSION="${VERSION:-latest}"
URL="https://github.com/wtanne20/Relay/releases/download/v${VERSION}/${ASSET}"
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
echo "Installed relay in $DEST."
if [ -t 0 ]; then
  echo
  echo "Now link this device to your brain."
  echo "On the brain: Settings -> Computers -> Link a computer shows a 6-digit code."
  read -r -p "Brain's Tailscale IP or name (e.g. 100.101.102.103): " BRAIN
  read -r -p "6-digit code: " CODE
  if [ -n "$BRAIN" ] && [ -n "$CODE" ]; then
    "$DEST/relay" link --brain "$BRAIN" --code "$CODE" --root "$ROOT" --pair-only
  fi
fi
echo
echo "To keep this device linked, run (in tmux, so it keeps going):"
echo
echo "  $DEST/start"
echo
echo "In Termux, run termux-wake-lock too, so Android doesn't pause it."
