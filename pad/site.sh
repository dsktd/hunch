#!/usr/bin/env bash
# Lay out the phone page in <dir>/pad/: the page, its fonts (the same files src/phone.rs serves on
# the LAN) and the iroh wasm module from pad/pkg.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=$1/pad
mkdir -p "$out/pkg" "$out/fonts"
cp "$root/assets/phone/index.html" "$out/"
cp "$root"/assets/fonts/*.ttf "$root"/assets/fonts/*-OFL.txt "$out/fonts/"
cp "$root"/pad/pkg/hunch_pad.js "$root"/pad/pkg/hunch_pad_bg.wasm "$out/pkg/"
