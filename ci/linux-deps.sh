#!/usr/bin/env bash
# Bevy's Linux build dependencies (Debian/Ubuntu).
set -euo pipefail
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  mold clang pkg-config libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev
