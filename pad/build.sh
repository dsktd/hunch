#!/usr/bin/env bash
# Build the phone link: the wasm module for the browser page and the native pad-host stand-in.
# Needs a rustup toolchain with wasm32-unknown-unknown (kept under ~/.cache/rustup).
# The host build links with clang+mold (.cargo/config.toml), so both come from nix here too.
set -euo pipefail
cd "$(dirname "$0")"
export RUSTUP_HOME=$HOME/.cache/rustup CARGO_HOME=$HOME/.cache/cargo-rustup
clang_unwrapped=$(nix build --no-link --print-out-paths nixpkgs#llvmPackages.clang-unwrapped)
llvm=$(nix build --no-link --print-out-paths nixpkgs#llvm)
exec nix shell nixpkgs#rustup nixpkgs#gcc nixpkgs#clang nixpkgs#mold nixpkgs#wasm-bindgen-cli -c bash -c "
  set -e
  cargo +stable build --release --bin pad-host -j${JOBS:-32}
  CC_wasm32_unknown_unknown=$clang_unwrapped/bin/clang AR_wasm32_unknown_unknown=$llvm/bin/llvm-ar \
    cargo +stable build --release --lib --target wasm32-unknown-unknown -j${JOBS:-32}
  wasm-bindgen --target web --no-typescript --out-dir pkg target/wasm32-unknown-unknown/release/hunch_pad.wasm
"
