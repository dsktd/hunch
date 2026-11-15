#!/usr/bin/env bash
# Steam Deck Game Mode entry: start the host, open its TV page full screen in a kiosk browser (Zen,
# a Firefox flatpak, with its own profile so it never joins a running browser); either side closing
# ends the other. Lives next to the host binary in ~/Games/hunch.
set -uo pipefail
cd "$(dirname "$0")"
log=host.log
: > "$log"
./hunch < /dev/null >> "$log" 2>&1 &
host=$!
trap 'kill "$host" 2>/dev/null; wait "$host" 2>/dev/null' EXIT
url=
for _ in $(seq 100); do
  url=$(sed -n 's/^TV: //p' "$log" | head -1)
  [ -n "$url" ] && break
  kill -0 "$host" 2>/dev/null || exit 1
  sleep 0.1
done
[ -n "$url" ] || exit 1
browser=${HUNCH_BROWSER:-app.zen_browser.zen}
profile=$HOME/.var/app/$browser/hunch-tv
mkdir -p "$profile"
flatpak run "$browser" --no-remote --profile "$profile" --kiosk "$url" &
# Leave when either side ends: the host quitting from its menu closes the browser too
while kill -0 "$host" 2>/dev/null && kill -0 $! 2>/dev/null; do sleep 0.5; done
flatpak kill "$browser" 2>/dev/null
