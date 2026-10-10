#!/bin/sh
# Builds libvorn_grid_ffi.a for the app: target/<target>/release/libvorn_grid_ffi.a
set -eu
target="${1:-aarch64-apple-darwin}"
cd "$(dirname "$0")"
rustup target add "$target" >/dev/null 2>&1 || true
nice -n 15 cargo build --release -j 4 --target "$target"
echo "$(pwd)/target/$target/release/libvorn_grid_ffi.a"
