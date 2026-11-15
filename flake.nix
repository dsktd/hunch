{
  description = "Hunch: a party trivia game";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      # The pinned toolchain from rust-toolchain.toml, the same one CI installs.
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      runtimeLibs = with pkgs; [
        alsa-lib
        udev
        vulkan-loader
        libxkbcommon
        wayland
      ];
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          toolchain
          pkg-config
          clang
          mold
          just
        ];
        buildInputs = runtimeLibs;
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
      };
    };
}
