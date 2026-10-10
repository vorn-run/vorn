#!/usr/bin/env bash
# Runs every measurement for one prototype binary into one directory:
#   scripts/measure.sh <binary> <name: vornui|gpui> <out dir>
# The vornd under test is .deps/bin (or VORN_SPIKE_BIN); each run starts its
# own in a fresh temp directory.
set -u
bin=$1
name=$2
out=$3
mkdir -p "$out"
fail=0
run() {
  echo "== $name $*"
  "$bin" "$@" || { echo "!! $name $1 failed"; fail=1; }
}
run shot --screen main --out "$out/$name-main.png"
run shot --screen main --scale 1 --out "$out/$name-main-1x.png"
run shot --screen main --scale 1.5 --out "$out/$name-main-1.5x.png"
run shot --screen grid --panes 8 --out "$out/$name-grid-8.png"
run shot --screen grid --panes 32 --out "$out/$name-grid-32.png"
run bench --panes 8 --out "$out/$name-bench-8.json"
run bench --panes 32 --out "$out/$name-bench-32.json"
run coldstart --out "$out/$name-coldstart.json"
run ime --out "$out/$name-ime.json"
run a11y --out "$out/$name-a11y.json"
exit $fail
