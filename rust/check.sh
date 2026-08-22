#!/usr/bin/env bash
# The one gate: formatting, lints, tests, and (once step 8 lands) the C
# demo. Run from anywhere; re-executes itself inside nix-shell so the
# toolchain always comes from the repo-root shell.nix (impl plan §8.12).
set -euo pipefail

cd "$(dirname "$0")"

if [ -z "${IN_SOIL_SHELL:-}" ]; then
    exec nix-shell ../shell.nix --pure \
        --run "IN_SOIL_SHELL=1 ./check.sh"
fi

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

if [ -x soil-rt/cdemo/build.sh ]; then
    soil-rt/cdemo/build.sh
fi
