#!/usr/bin/env bash
# Adapted from Gobble Grand Prix (Unlicense).
# Publish the phone page to GitHub Pages (gh-pages branch, /pad/): the same page the game
# serves on the LAN, its font and glyphs, plus the iroh wasm module so it can reach the game from any network.
# Run pad/build.sh first.
set -euo pipefail
cd "$(dirname "$0")/.."
site=$(mktemp -d)
trap 'rm -rf "$site"' EXIT
pad/site.sh "$site"
printf '<meta http-equiv="refresh" content="0; url=pad/">\n' > "$site/index.html"
touch "$site/.nojekyll"
commit=$(git rev-parse --short HEAD)
git -C "$site" init -q -b gh-pages
git -C "$site" add -A
git -C "$site" -c user.name="$(git config user.name)" -c user.email="$(git config user.email)" \
  commit -qm "Phone pad page from $commit"
git -C "$site" -c credential.helper= -c credential.helper="!$HOME/claude/_tools/claude-wrapper/bin/ghd auth git-credential" push -qf "$(git remote get-url origin)" gh-pages
echo "published https://dsktd.github.io/hunch/pad/"
