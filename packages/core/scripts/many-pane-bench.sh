#!/bin/sh
# The many-pane benchmark in one command: builds the test daemons, the CLI
# (vornd compiles linked Wasm components with it) and the bench (ci profile),
# then runs it. Arguments go to the bench, e.g. --panes 8,16 --extensions 0,20.
set -eu
cd "$(dirname "$0")/.."
cargo build --profile ci --locked -p vornd -p vorn-sessiond -p vorn-cli --bins
cargo build --profile ci --locked -p vorn-app --example many_panes
exec target/ci/examples/many_panes "$@"
