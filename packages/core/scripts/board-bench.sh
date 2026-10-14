#!/bin/sh
# The Tasks board benchmark in one command: builds the test daemons and the
# bench (ci profile), then runs it. Arguments go to the bench, e.g. --tiers 100,1000.
set -eu
cd "$(dirname "$0")/.."
cargo build --profile ci --locked -p vornd -p vorn-sessiond
cargo build --profile ci --locked -p vorn-app --example board
exec target/ci/examples/board "$@"
