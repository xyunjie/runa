#!/usr/bin/env bash
# Render GUI screenshots in headless Chrome with mocked IPC (gui/preview/mock.js).
# Usage: scripts/ui-screens.sh [out_dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/screens}"
CHROME="${CHROME:-$(command -v google-chrome || command -v chromium || command -v chromium-browser)}"
TMP="$(mktemp -d)"
cp -r "$ROOT/gui/ui/." "$TMP/"
cp "$ROOT/gui/preview/mock.js" "$TMP/"
sed -i.bak 's#<script type="module" src="app.js"></script>#<script src="mock.js"></script><script type="module" src="app.js"></script>#' "$TMP/index.html"
PORT="${PORT:-8765}"
python3 -m http.server "$PORT" --directory "$TMP" >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV; rm -rf "$TMP"' EXIT
sleep 0.5
mkdir -p "$OUT"
shot() { # name query hash [budget]
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 --window-size=1280,820 \
    --virtual-time-budget="${4:-2500}" --screenshot="$OUT/$1.png" "http://127.0.0.1:$PORT/index.html?static=1&$2#$3" >/dev/null 2>&1
  echo "$OUT/$1.png"
}
for theme in light dark; do
  shot "overview-$theme" "theme=$theme" "/overview"
  shot "tool-installed-$theme" "theme=$theme" "/tool/node"
  shot "tool-available-$theme" "theme=$theme" "/tool/go"
  shot "tool-search-$theme" "theme=$theme&demo=search&q=1.2" "/tool/go"
  shot "installing-$theme" "theme=$theme&demo=install" "/tool/go" 4000
  shot "detect-$theme" "theme=$theme&demo=detect" "/detect"
  shot "settings-$theme" "theme=$theme" "/settings"
done
shot "overview-not-activated-light" "theme=light&activated=0" "/overview"
shot "confirm-uninstall-light" "theme=light&demo=confirm" "/tool/node" 6000
shot "loading-skeleton-dark" "theme=dark&slow=1" "/overview" 800
shot "remote-error-light" "theme=light&remote=error" "/tool/go"
shot "empty-state-light" "theme=light&empty=1&demo=tab-installed" "/tool/java"
shot "empty-state-dark" "theme=dark&empty=1&demo=tab-installed" "/tool/python"
