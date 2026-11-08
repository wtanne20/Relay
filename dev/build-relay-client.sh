#!/usr/bin/env bash
# Build the Relay Client .deb for x86_64 Linux.
#
# Packages the relay binary + a launcher + a menu entry into a .deb that
# installs on a laptop. After installing, the user runs "Relay" from the app
# menu (or `relay-client`); on first run it asks for the brain's address +
# token, starts the host (exposing this machine's files + shell), pairs it
# with the brain, and opens the UI.
#
# Inputs (relative to the repo root):
#   server/target/release/relay        the x86_64 relay binary (already built)
#   dev/relay-client/relay-client      the launcher script
#   dev/relay-client/relay-client.desktop
#   dev/relay-client/relay-host.service
#
# Output:
#   dist/RelayClient-<version>-amd64.deb

set -euo pipefail
cd "$(dirname "$0")/.."   # repo root

VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' server/Cargo.toml | head -n1)"
VERSION="${VERSION:-dev}"
PKG="relay-client"
ARCH="amd64"
OUT="dist/RelayClient-${VERSION}-${ARCH}.deb"

RELAY_BIN="server/target/release/relay"
SRC="dev/relay-client"

[ -f "$RELAY_BIN" ] || { echo "missing $RELAY_BIN (build the server first)"; exit 1; }
[ -f "$SRC/relay-client" ] || { echo "missing $SRC/relay-client"; exit 1; }

ROOT="dist/${PKG}-${VERSION}"
rm -rf "$ROOT"
mkdir -p "$ROOT/DEBIAN" \
         "$ROOT/usr/local/bin" \
         "$ROOT/usr/local/share/applications" \
         "$ROOT/usr/local/lib/systemd/user"

# Payload.
install -m 0755 "$RELAY_BIN" "$ROOT/usr/local/bin/relay"
install -m 0755 "$SRC/relay-client" "$ROOT/usr/local/bin/relay-client"
install -m 0644 "$SRC/relay-client.desktop" "$ROOT/usr/local/share/applications/relay-client.desktop"
install -m 0644 "$SRC/relay-host.service" "$ROOT/usr/local/lib/systemd/user/relay-host.service"

# Control file.
cat > "$ROOT/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Architecture: $ARCH
Maintainer: Relay <relay@localhost>
Section: utils
Priority: optional
Depends: curl, xdg-utils
Description: Relay Client
 Connect a Linux machine to a Relay brain. Installs the relay binary and a
 "Relay" app that starts this machine's host (exposing its files and shell),
 pairs it with the brain, and opens the UI. The brain (on another PC) does
 all the model processing; this machine is the "hands".
EOF

# postinst: refresh the desktop menu (best-effort).
cat > "$ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
command -v update-desktop-database >/dev/null 2>&1 && \
  update-desktop-database /usr/local/share/applications 2>/dev/null || true
exit 0
EOF
chmod 0755 "$ROOT/DEBIAN/postinst"

# Build.
mkdir -p dist
rm -f "$OUT"
dpkg-deb --build --root-owner-group "$ROOT" "$OUT"
ls -la "$OUT"
echo "Built: $OUT"
