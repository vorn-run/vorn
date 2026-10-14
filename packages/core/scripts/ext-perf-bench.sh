#!/bin/sh
# Tasks and Workflows built in against the same work as extensions, in one command; arguments go to the bench, e.g. --reps 5 --only tasks,board.
set -eu
cd "$(dirname "$0")/.."
out=${EXT_PERF_OUT:-bench/ext-perf/results/$(date +%Y-%m-%d)-$(git rev-parse --short HEAD)}
mkdir -p "$out"
(cd bench/ext-perf/component && cargo build --release --locked --target wasm32-wasip2)
# vornd on its own, so it links Wasmtime's runtime only and not the bench's compiler.
cargo build --profile ci --locked -p vornd --features ext-spike
cargo build --profile ci --locked -p vorn-sessiond -p vorn-ext-perf-bench
exec target/ci/vorn-ext-perf run --wasm bench/ext-perf/component/target/wasm32-wasip2/release --bin target/ci --out "$out" "$@"
