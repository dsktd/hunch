#!/usr/bin/env bash
# Build the host on the desktop and stage it on the Steam Deck with progress in the Deck overlay
# (deckjob): build, copy, then a swap that checks the hash, moves the new binary in atomically (a
# running game keeps its copy, the next launch gets the new one), checks ldd, refreshes the
# launcher and writes BUILD.txt. Never launches or kills the game.
#   deck/deploy.sh [--release]
set -euo pipefail
cd "$(dirname "$0")/.."
D=~/claude/.services/deckupdate/deckjob
dk=~/claude/nx2navi/tools/deck/dk
profile=deck-dev
[ "${1:-}" = "--release" ] && profile=release
bin=target/deck/x86_64-unknown-linux-gnu/$profile/hunch
commit=$(git describe --always --dirty)

stage() {
  "$D" cargo "Hunch" -- ./deck-build.sh "$@" || return 1
  local sum built launcher
  sum=$(sha256sum "$bin" | cut -d' ' -f1)
  built=$(date -u -r "$bin" +%FT%TZ)
  launcher=$(base64 -w0 deck/hunch-tv.sh)
  "$D" copy "Hunch" "$bin" deck:/tmp/hunch.new || return 1
  "$D" run "Hunch" swap -- "$dk" "
    set -e
    mkdir -p ~/Games/hunch
    cd ~/Games/hunch
    echo '$launcher' | base64 -d > hunch-tv.sh.new && chmod 755 hunch-tv.sh.new && mv hunch-tv.sh.new hunch-tv.sh
    install -m755 /tmp/hunch.new hunch.new
    [ \"\$(sha256sum hunch.new | cut -d' ' -f1)\" = '$sum' ]
    ! ldd hunch.new | grep 'not found'
    mv hunch.new hunch
    printf 'commit %s\nprofile %s\nbuilt %s\nsha256 %s\n' '$commit' '$profile' '$built' '${sum:0:16}' > BUILD.txt
    cat BUILD.txt" || return 1
}

if stage "$@"; then
  "$D" "done"
else
  "$D" fail
  exit 1
fi
