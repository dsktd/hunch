#!/usr/bin/env bash
# Steam Deck build of the host: glibc 2.35 floor via zig, Zen 2 codegen, in its own target dir so
# these flags never invalidate the desktop cache. Runs on the desktop; the Deck never builds.
#   ./deck-build.sh            iterative test build (deck-dev profile)
#   ./deck-build.sh --release  tagged release build (release profile)
# Binary: target/deck/x86_64-unknown-linux-gnu/<profile>/hunch
set -euo pipefail
cd "$(dirname "$0")"
profile=deck-dev
[ "${1:-}" = "--release" ] && profile=release
export RUSTFLAGS="-C target-cpu=znver2"
export CARGO_TARGET_DIR="$PWD/target/deck"
exec nice -n 19 nix develop --command nix shell nixpkgs#cargo-zigbuild nixpkgs#zig -c \
  cargo zigbuild --profile "$profile" -j"${JOBS:-16}" --target x86_64-unknown-linux-gnu.2.35 --bin hunch
