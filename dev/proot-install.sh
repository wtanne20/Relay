#!/usr/bin/env bash
# Install the Relay desktop app in Proot on an Android phone (shown through
# Termux:X11 or VNC). Run it from the unpacked release folder:
#
#   tar xzf Relay-<version>-arm64-proot.tar.gz
#   cd Relay-<version>-arm64-proot
#   ./install.sh
#
# What it does:
#   - installs the libraries the app needs (WebKitGTK, GTK) with apt or pacman
#   - copies relay-desktop to ~/.local/bin and adds Relay to the app menu
#
# Then start it with `relay-desktop` from the X11 session. On first launch it
# asks whether this computer is the brain or linked to a brain elsewhere.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${RELAY_BIN_DIR:-$HOME/.local/bin}"
APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"

if [ ! -f "$HERE/relay-desktop" ]; then
  echo "relay-desktop is not next to install.sh; run this from the unpacked release folder." >&2
  exit 1
fi

SUDO=""
if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
  SUDO="sudo"
fi

echo "Installing the libraries Relay needs"
if command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update
  $SUDO apt-get install -y --no-install-recommends \
    libwebkit2gtk-4.1-0 libgtk-3-0 librsvg2-common libayatana-appindicator3-1
elif command -v pacman >/dev/null 2>&1; then
  $SUDO pacman -S --needed --noconfirm webkit2gtk-4.1 gtk3 librsvg libayatana-appindicator
else
  echo "No apt or pacman here; install WebKitGTK 4.1 and GTK 3 yourself, then run this again." >&2
fi

mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICON_DIR"
install -m 0755 "$HERE/relay-desktop" "$BIN_DIR/relay-desktop"
if [ -f "$HERE/relay.png" ]; then
  install -m 0644 "$HERE/relay.png" "$ICON_DIR/relay-desktop.png"
fi
cat > "$APPS_DIR/relay.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Relay
Comment=Your local AI agent
Exec="$BIN_DIR/relay-desktop"
Icon=relay-desktop
Terminal=false
Categories=Development;
EOF

echo
echo "Installed Relay in $BIN_DIR."
case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Start it from the app menu, or run: relay-desktop" ;;
  *) echo "Start it from the app menu, or run: $BIN_DIR/relay-desktop" ;;
esac
