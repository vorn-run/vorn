#!/bin/sh
# Measures the Widget, Worktree cleanup and RSS as Wasm components and as
# programs on the same manifest, and RSS as the Node connector Vorn ships.
#   yarn bench:ext-tiers [--reps N] [--out DIR]
set -e
cd "$(dirname "$0")/.."
cargo build --profile ci --locked -p vorn-bench-ext-tiers
exec target/ci/ext-tiers "$@"
