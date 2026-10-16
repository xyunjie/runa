#!/usr/bin/env bash
# Render GUI screenshots in headless Chrome. Outside Tauri the UI uses its IPC mock
# (gui/web/src/lib/mock.ts), so the built frontend can be served as plain files.
# Usage: scripts/ui-screens.sh [out_dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/screens}"
CHROME="${CHROME:-$(command -v google-chrome || command -v chromium || command -v chromium-browser)}"
(cd "$ROOT/gui/web" && { [ -d node_modules ] || npm ci; } && npm run build >/dev/null)
PORT="${PORT:-8765}"
python3 -m http.server "$PORT" --directory "$ROOT/gui/web/dist" >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV' EXIT
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
  shot "detect-$theme" "theme=$theme" "/detect"
  shot "managers-$theme" "theme=$theme" "/managers"
  shot "settings-$theme" "theme=$theme" "/settings"
done
shot "overview-not-activated-light" "theme=light&activated=0" "/overview"
shot "loading-skeleton-dark" "theme=dark&slow=1" "/overview" 800
shot "remote-error-light" "theme=light&remote=error" "/tool/go"
shot "empty-state-light" "theme=light&empty=1" "/tool/java"
