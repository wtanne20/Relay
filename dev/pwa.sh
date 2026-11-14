#!/bin/sh
# Open the Relay brain's web UI (the PWA) in a browser.
#
#   ./pwa http://your-pc:8765
#   RELY_BRAIN=http://your-pc:8765 ./pwa
#
# The brain serves the same UI as the Windows desktop app. Open it in a
# browser and use "Add to Home Screen" / "Install app" to make it a
# full-screen app. The brain's address is the one you paired with.
set -eu
URL="${1:-${RELY_BRAIN:-}}"
if [ -z "$URL" ]; then
  echo "Usage: pwa http://your-pc:8765" >&2
  echo "(or set RELY_BRAIN=http://your-pc:8765)" >&2
  exit 1
fi
case "$URL" in
  http://*|https://*) : ;;
  *) URL="http://$URL" ;;
esac
echo "Opening $URL in your browser..."
echo "Then use 'Add to Home Screen' / 'Install app' to make it a full-screen app."
if command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$URL" >/dev/null 2>&1 &
elif command -v open >/dev/null 2>&1; then
  open "$URL" >/dev/null 2>&1 &
else
  echo "No browser launcher found. Open this URL in your browser:"
  echo "  $URL"
fi
