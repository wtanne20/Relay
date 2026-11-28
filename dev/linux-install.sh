#!/usr/bin/env bash
# Install (or update) the Relay desktop app on an x64 Linux desktop, such as
# Arch. Each release ships this script as install-relay-linux.sh.
#
#   ./install-relay-linux.sh                       # newest Relay-*-x86_64.AppImage
#                                                  # next to this script, here or in
#                                                  # ~/Downloads; else download it
#   ./install-relay-linux.sh Relay-1.0.16-x86_64.AppImage
#   ./install-relay-linux.sh --download            # always fetch the latest release
#   ./install-relay-linux.sh --uninstall           # remove the app (keeps your data)
#
# Or straight from GitHub:
#
#   curl -fsSL https://github.com/wtanne20/Relay/releases/latest/download/install-relay-linux.sh | bash
#
# What it does, the same way the app does it when it installs itself:
#   - copies the AppImage to ~/Applications/Relay.AppImage, the file Relay's
#     updater replaces when a new version comes out
#   - adds Relay with its icon to the app menu
#     (~/.local/share/applications/relay-desktop.desktop)
#   - points an existing "start on login" entry at the installed copy
#
# Nothing here needs root. Your chats, models and settings
# (~/.local/share/relay) are not touched.

set -euo pipefail

REPO="wtanne20/Relay"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
# Must match installed_appimage() in desktop/src-tauri/src/main.rs.
INSTALL_DIR="$HOME/Applications"
TARGET="$INSTALL_DIR/Relay.AppImage"
APPS_DIR="$DATA_HOME/applications"
ENTRY="$APPS_DIR/relay-desktop.desktop"
ICONS_DIR="$DATA_HOME/icons/hicolor"
AUTOSTART="$CONFIG_HOME/autostart/relay.desktop"
STAMP="$DATA_HOME/relay/appimage-version"

say() { printf '%s\n' "$*"; }
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

# The folder this script is in; empty when it is piped into bash.
HERE=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

usage() {
  if [ -n "$HERE" ]; then
    sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  else
    say "Usage: install-relay-linux.sh [Relay-<version>-x86_64.AppImage | --download | --uninstall]"
  fi
}

refresh_menus() {
  # Most desktops notice on their own; these nudge the rest.
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$ICONS_DIR/index.theme" ]; then
    gtk-update-icon-cache -q -t "$ICONS_DIR" >/dev/null 2>&1 || true
  fi
}

uninstall() {
  rm -f "$TARGET" "$INSTALL_DIR/.Relay.AppImage.part" "$ENTRY" "$AUTOSTART" "$STAMP"
  for size in 32x32 128x128 256x256 512x512; do
    rm -f "$ICONS_DIR/$size/apps/relay-desktop.png"
  done
  refresh_menus
  say "Removed Relay. Your data in $DATA_HOME/relay is still there; delete that folder to remove it too."
}

# Newest Relay-<version>-x86_64.AppImage in the usual places, or nothing.
find_local() {
  local dirs=() d
  [ -n "$HERE" ] && dirs+=("$HERE")
  dirs+=("$PWD")
  [ -n "${XDG_DOWNLOAD_DIR:-}" ] && dirs+=("$XDG_DOWNLOAD_DIR")
  dirs+=("$HOME/Downloads")
  for d in "${dirs[@]}"; do
    [ -d "$d" ] || continue
    find "$d" -maxdepth 1 -type f -name 'Relay-*-x86_64.AppImage' 2>/dev/null
  done | awk '!seen[$0]++' | sort -V | tail -n 1
}

# Downloads the latest release's AppImage into $1 and prints its path. The
# file is checked against the SHA-256 GitHub lists for it, like the updater.
download_latest() {
  local dir="$1" json name="" url sha="" file
  command -v curl >/dev/null 2>&1 || die "curl is needed to download Relay. Install it, or download the AppImage yourself and pass its path."
  say "Looking up the latest Relay release" >&2
  json="$(curl -fsSL -H 'Accept: application/vnd.github+json' "https://api.github.com/repos/$REPO/releases/latest")" \
    || die "could not reach GitHub to find the latest release."
  # Each asset lists its "name" before its "digest" (the uploader has no
  # "name"), so pair them up in order. No jq needed.
  local tag
  tag="$(printf '%s' "$json" | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')"
  read -r name sha < <(printf '%s' "$json" \
    | grep -oE '"(name|digest)"[[:space:]]*:[[:space:]]*"[^"]*"' \
    | sed -E 's/^"([a-z]+)"[[:space:]]*:[[:space:]]*"([^"]*)"$/\1 \2/' \
    | awk '$1 == "name" { if (n != "") exit; if ($2 ~ /^Relay-.*-x86_64\.AppImage$/) n = $2; next }
           $1 == "digest" && n != "" { d = $2; exit }
           END { if (n != "") { sub(/^sha256:/, "", d); print n, d } }') || true
  [ -n "$tag" ] && [ -n "${name:-}" ] || die "the latest release has no x64 AppImage."
  url="https://github.com/$REPO/releases/download/$tag/$name"
  file="$dir/$name"
  say "Downloading $name" >&2
  curl -fL --progress-bar -o "$file" "$url" >&2 || die "the download failed."
  if [ -n "$sha" ]; then
    command -v sha256sum >/dev/null 2>&1 || die "sha256sum is needed to check the download."
    local got
    got="$(sha256sum "$file" | cut -d' ' -f1)"
    [ "${got,,}" = "${sha,,}" ] || die "the download is damaged (SHA-256 doesn't match). Try again."
  else
    say "Warning: GitHub listed no SHA-256 for $name, so the download was not checked." >&2
  fi
  printf '%s\n' "$file"
}

# Saves Relay's icons from inside the AppImage, or from relay.png next to
# this script. Without either, the app adds them when it first starts.
install_icons() {
  local appimage="$1" tmp found=0 size png
  tmp="$(mktemp -d)"
  # --appimage-extract unpacks into ./squashfs-root, so run it in $tmp.
  if (cd "$tmp" && "$appimage" --appimage-extract 'usr/share/icons/*' >/dev/null 2>&1) \
    || (cd "$tmp" && "$appimage" --appimage-extract '*.png' >/dev/null 2>&1); then
    for size in 32x32 128x128 256x256 256x256@2 512x512; do
      png="$tmp/squashfs-root/usr/share/icons/hicolor/$size/apps/relay-desktop.png"
      [ -f "$png" ] || continue
      local dest="${size%@2}"
      mkdir -p "$ICONS_DIR/$dest/apps"
      install -m 0644 "$png" "$ICONS_DIR/$dest/apps/relay-desktop.png"
      found=1
    done
    if [ "$found" = 0 ]; then
      png="$(find "$tmp/squashfs-root" -maxdepth 1 -name '*.png' -print -quit 2>/dev/null || true)"
      if [ -n "$png" ]; then
        mkdir -p "$ICONS_DIR/128x128/apps"
        install -m 0644 "$png" "$ICONS_DIR/128x128/apps/relay-desktop.png"
        found=1
      fi
    fi
  fi
  rm -rf "$tmp"
  if [ "$found" = 0 ] && [ -n "$HERE" ] && [ -f "$HERE/relay.png" ]; then
    mkdir -p "$ICONS_DIR/128x128/apps"
    install -m 0644 "$HERE/relay.png" "$ICONS_DIR/128x128/apps/relay-desktop.png"
    found=1
  fi
  [ "$found" = 1 ] || say "Note: couldn't read Relay's icon from the AppImage; Relay adds it when it starts."
}

# Quoted for the Exec key: backslash, quote, backtick and $ are escaped.
exec_quote() {
  local s="$1"
  s="${s//\\/\\\\}"; s="${s//\"/\\\"}"; s="${s//\`/\\\`}"; s="${s//\$/\\\$}"
  printf '"%s"' "$s"
}

# The same entry desktop_entry() in main.rs writes, so the app leaves it be.
write_entry() {
  local exec
  exec="$(exec_quote "$TARGET")"
  mkdir -p "$APPS_DIR"
  printf '[Desktop Entry]\nType=Application\nName=Relay\nComment=Your local AI agent\nExec=%s\nIcon=relay-desktop\nTerminal=false\nCategories=Development;Utility;\nStartupWMClass=relay-desktop\n' \
    "$exec" > "$ENTRY.tmp"
  chmod 0644 "$ENTRY.tmp"
  mv -f "$ENTRY.tmp" "$ENTRY"
  # Start on login was turned on in the app: run the installed copy.
  if [ -f "$AUTOSTART" ]; then
    printf '[Desktop Entry]\nType=Application\nName=Relay\nComment=Keeps this computer linked to your Relay brain\nExec=%s --minimized\nIcon=relay-desktop\nTerminal=false\nX-GNOME-Autostart-enabled=true\n' \
      "$exec" > "$AUTOSTART.tmp"
    mv -f "$AUTOSTART.tmp" "$AUTOSTART"
  fi
}

check_fuse() {
  # AppImages mount themselves with FUSE 2.
  if command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    return
  fi
  for f in /usr/lib/libfuse.so.2 /usr/lib64/libfuse.so.2 /usr/lib/x86_64-linux-gnu/libfuse.so.2 /lib/x86_64-linux-gnu/libfuse.so.2; do
    [ -e "$f" ] && return
  done
  say ""
  say "Heads up: FUSE 2 (libfuse.so.2) wasn't found, and Relay needs it to start."
  if command -v pacman >/dev/null 2>&1; then
    say "  Install it with: sudo pacman -S fuse2"
  elif command -v apt-get >/dev/null 2>&1; then
    say "  Install it with: sudo apt install libfuse2   (libfuse2t64 on Ubuntu 24.04+)"
  elif command -v dnf >/dev/null 2>&1; then
    say "  Install it with: sudo dnf install fuse-libs"
  else
    say "  Install your distribution's FUSE 2 package."
  fi
}

main() {
  local src="" download=0
  while [ $# -gt 0 ]; do
    case "$1" in
      -h|--help) usage; exit 0 ;;
      --uninstall) uninstall; exit 0 ;;
      --download) download=1 ;;
      -*) die "unknown option $1 (see --help)" ;;
      *) [ -z "$src" ] || die "give one AppImage at most"; src="$1" ;;
    esac
    shift
  done

  [ "$(uname -s)" = Linux ] || die "this installer is for Linux."
  case "$(uname -m)" in
    x86_64|amd64) ;;
    aarch64|arm64) die "this is the x64 installer. On an arm64 phone in Proot, use the install.sh inside Relay-<version>-arm64-proot.tar.gz." ;;
    *) die "Relay's AppImage is built for x86_64, not $(uname -m)." ;;
  esac
  if [ "$(id -u)" -eq 0 ] && [ -z "${RELAY_ALLOW_ROOT:-}" ]; then
    die "run this as your own user, not root (Relay installs into your home folder)."
  fi

  TMPDIR_RELAY=""
  trap '[ -n "${TMPDIR_RELAY:-}" ] && rm -rf "$TMPDIR_RELAY"' EXIT
  if [ -z "$src" ] && [ "$download" = 0 ]; then
    src="$(find_local)"
    if [ -n "$src" ]; then
      say "Found $src"
      # An old download lying around shouldn't replace a newer install.
      local found installed=""
      found="$(basename "$src")"; found="${found#Relay-}"; found="${found%-x86_64.AppImage}"
      [ -f "$TARGET" ] && [ -f "$STAMP" ] && installed="$(tr -d '[:space:]' < "$STAMP")"
      if [ -n "$installed" ] && [ "$installed" != "$found" ] \
        && [ "$(printf '%s\n%s\n' "$found" "$installed" | sort -V | tail -n 1)" = "$installed" ]; then
        say "Relay $installed is already installed, newer than that file; getting the latest release instead."
        src=""
      fi
    fi
  fi
  if [ -z "$src" ]; then
    TMPDIR_RELAY="$(mktemp -d)"
    src="$(download_latest "$TMPDIR_RELAY")"
  fi
  [ -f "$src" ] || die "$src not found."
  src="$(cd "$(dirname "$src")" && pwd)/$(basename "$src")"
  # An AppImage is an ELF program.
  [ "$(head -c 4 "$src" | od -An -c | tr -d ' ')" = '177ELF' ] || die "$src is not an AppImage."

  local version=""
  case "$(basename "$src")" in
    Relay-*-x86_64.AppImage) version="$(basename "$src")"; version="${version#Relay-}"; version="${version%-x86_64.AppImage}" ;;
  esac

  mkdir -p "$INSTALL_DIR"
  if [ "$src" -ef "$TARGET" ]; then
    say "Relay is already at $TARGET; refreshing the app menu entry."
  else
    # Copy next to the target, then rename: a running Relay keeps its old
    # file, and the swap is atomic (the updater does the same).
    local part="$INSTALL_DIR/.Relay.AppImage.part"
    cp -f "$src" "$part"
    chmod 0755 "$part"
    mv -f "$part" "$TARGET"
    say "Installed ${version:+Relay $version }at $TARGET"
    # Tell the app which version is installed, so a launch of an older
    # download doesn't copy itself over this one.
    if [ -n "$version" ]; then
      mkdir -p "$(dirname "$STAMP")"
      printf '%s' "$version" > "$STAMP"
    fi
  fi
  chmod 0755 "$TARGET"

  install_icons "$TARGET"
  write_entry
  refresh_menus
  say "Added Relay to the app menu ($ENTRY)"

  if pgrep -x relay-desktop >/dev/null 2>&1; then
    say "Relay is running; quit it from the tray and open it again to use this version."
  fi
  check_fuse
  say ""
  say "Done. Open Relay from your app menu, or run: $TARGET"
}

main "$@"
