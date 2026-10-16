#!/bin/sh
# The extension tiers benchmark (E9.4) in one command: builds the program, the
# Wasm component, the bench and the size probe (release, as vornd ships), then
# runs every tier and writes bench/ext-tiers/results/{results.json,summary.md}
# and the widget fixture vorn-app's templates test draws. Arguments go to the
# bench, e.g. --runs 3.
#
# `ext-tiers-bench.sh tasks` runs Tasks instead (the built-in, a program and a
# component over one board) into bench/ext-tiers/results/tasks, e.g.
# `tasks --reps 5 --tiers 100,1000,10000`.
set -eu
cd "$(dirname "$0")/.."
rustup target list --installed | grep -qx wasm32-wasip2 || rustup target add wasm32-wasip2
cargo build --release --locked -p ext-tiers-program -p vorn-bench-ext-tiers
cargo build --release --locked --target wasm32-wasip2 -p ext-tiers-component
target=${CARGO_TARGET_DIR:-target}
prettier=../../node_modules/.bin/prettier
if [ "${1:-}" = tasks ]; then
  shift
  target=$(cd "$target" && pwd)
  component="$target/wasm32-wasip2/release/ext_tiers_component.wasm"
  "$target/release/ext-tiers" precompile "$component"
  "$target/release/ext-tiers" tasks \
    --program "$target/release/ext-tiers-program" \
    --component "$component" \
    --out bench/ext-tiers/results/tasks \
    "$@"
  [ ! -x "$prettier" ] || "$prettier" --log-level warn --write bench/ext-tiers/results/tasks
  cat bench/ext-tiers/results/tasks/summary.md
  exit
fi
# The host three times: without the Wasm tier, with its runtime (what vornd
# ships) and with the compiler too; the differences are what each adds.
cargo build --release --locked -p ext-tiers-probe
cp "$target/release/ext-tiers-probe" "$target/release/ext-tiers-probe-off"
cargo build --release --locked -p ext-tiers-probe --features wasm
cp "$target/release/ext-tiers-probe" "$target/release/ext-tiers-probe-on"
cargo build --release --locked -p ext-tiers-probe --features wasm-compile
cp "$target/release/ext-tiers-probe" "$target/release/ext-tiers-probe-compile"
target=$(cd "$target" && pwd)
component="$target/wasm32-wasip2/release/ext_tiers_component.wasm"
bench="$target/release/ext-tiers"
"$bench" precompile "$component"
"$bench" all \
  --program "$target/release/ext-tiers-program" \
  --component "$component" \
  --out bench/ext-tiers/results \
  --fixture bench/ext-tiers/fixtures/widget-asking.json \
  --probe-off "$target/release/ext-tiers-probe-off" \
  --probe-on "$target/release/ext-tiers-probe-on" \
  --probe-compile "$target/release/ext-tiers-probe-compile" \
  "$@"
node bench/ext-tiers/summary.mjs bench/ext-tiers/results/results.json > bench/ext-tiers/results/summary.md
[ ! -x "$prettier" ] || "$prettier" --log-level warn --write bench/ext-tiers/results bench/ext-tiers/fixtures
cat bench/ext-tiers/results/summary.md
