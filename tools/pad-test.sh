#!/usr/bin/env bash
# Run the phone tests locally with Playwright and browsers from nixpkgs (the bundle has the Chromium
# headless shell, not full Chromium). Run pad/build.sh first; play needs `cargo build --release`.
#   tools/pad-test.sh [screenshot dir]   join over iroh + every screen's layout
#   tools/pad-test.sh play              a whole match, real host, three phones on the LAN page
#   tools/pad-test.sh tv [shot dir]     one round driven from the TV page, two phones
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
site=$(mktemp -d)
trap 'rm -rf "$site"' EXIT
[ "${1:-}" = tv ] || "$root/pad/site.sh" "$site"
export PLAYWRIGHT_BROWSERS_PATH=$(nix eval --raw nixpkgs#playwright-driver.browsers.outPath)
export PAD_CHROMIUM_CHANNEL=""
# The container has no system fonts; give the browsers a fallback so text has real metrics
export FONTCONFIG_FILE=$(nix build --no-link --print-out-paths --impure --expr \
  'with import (builtins.getFlake "nixpkgs") {}; makeFontsConf { fontDirectories = [ dejavu_fonts ]; }')
if [ "${1:-}" = tv ]; then
  test=(tools/tv_play_test.py "$root/target/release/hunch" "${@:2}")
elif [ "${1:-}" = play ]; then
  test=(tools/lan_play_test.py "$root/target/release/hunch")
else
  test=(tools/pad_join_test.py "$root/pad/target/release/pad-host" "$site" "$@")
fi
exec nix shell --impure --expr 'with import (builtins.getFlake "nixpkgs") {}; [ (python3.withPackages (p: [ p.playwright ])) ]' \
  -c python3 "${test[0]/#tools/$root/tools}" "${test[@]:1}"
