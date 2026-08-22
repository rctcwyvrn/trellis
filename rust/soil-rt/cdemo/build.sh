#!/usr/bin/env bash
# Build and run the C demo against the staticlib. Invoked by check.sh
# inside nix-shell; the demo passing (exit 0) is the embeddability exit
# criterion of plan 01.
set -euo pipefail

cd "$(dirname "$0")"

cargo build -p soil-rt
cbindgen .. --config ../cbindgen.toml --output soil_rt.h

cc -std=c11 -Wall -Wextra -Werror main.c \
    ../../target/debug/libsoil_rt.a \
    -lpthread -ldl -lm \
    -o cdemo_bin

./cdemo_bin
