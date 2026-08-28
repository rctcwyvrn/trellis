# The dev environment for the whole repo — the single source of truth for
# every tool (impl plan 01 §8.12): the Rust toolchain (no
# rust-toolchain.toml; this nixpkgs pin determines rustc), cbindgen, and a
# C compiler for the soil-rt demo. Later milestones add their tools here
# (Z3, Python). Building outside `nix-shell` is off the supported path.
#
# Rust *crate* dependencies are managed by Cargo, not this file.
{ pkgs ? import (builtins.fetchTarball {
    # nixos-25.05 as of 2026-08-22; bump by taking a new rev + hash.
    url = "https://github.com/NixOS/nixpkgs/archive/ac62194c3917d5f474c1a844b6fd6da2db95077d.tar.gz";
    sha256 = "0v6bd1xk8a2aal83karlvc853x44dg1n4nk08jg3dajqyy0s98np";
  }) {} }:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    rust-analyzer
    rust-cbindgen
    gcc
    # Plan 03 (the daemon): the OS jail around the lowering agent, and
    # the interim differential-oracle runner (impl plan 03 step 2).
    bubblewrap
    python3
  ];
}
