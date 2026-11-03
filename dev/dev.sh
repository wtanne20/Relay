#!/usr/bin/env bash
# Relay dev environment for Linux/macOS (same as dev.ps1).
#   ./dev/dev.sh [--mock] [--no-desktop]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
mock=0; desktop=1
for a in "$@"; do
  case "$a" in
    --mock) mock=1 ;;
    --no-desktop) desktop=0 ;;
    *) echo "unknown option $a"; exit 2 ;;
  esac
done
if [ "$mock" = 1 ]; then
  data="$root/dev/data"; mkdir -p "$data"
  if [ ! -f "$data/config.toml" ]; then
    (cd "$root/server" && cargo build --example mock_llama)
    printf GGUF > "$data/model.gguf"
    sed "s#@MOCK_LLAMA@#$root/server/target/debug/examples/mock_llama#; s#@MODEL@#$data/model.gguf#" \
      "$root/dev/mock-config.toml" > "$data/config.toml"
  fi
  export RELAY_CONFIG="$data/config.toml"
  echo "Mock mode: server on port 8766, data in $data"
fi
if command -v tailscale >/dev/null 2>&1; then
  REACT_NATIVE_PACKAGER_HOSTNAME="$(tailscale ip -4 2>/dev/null | head -n1)"; export REACT_NATIVE_PACKAGER_HOSTNAME
fi
(cd "$root/mobile" && npx expo start --dev-client) &
metro=$!
trap 'kill $metro 2>/dev/null || true' EXIT
if [ "$desktop" = 1 ]; then
  (cd "$root/desktop" && npx tauri dev --additional-watch-folders "$root/server/src")
else
  wait $metro
fi
