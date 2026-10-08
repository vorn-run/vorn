#!/bin/sh
# The measurement matrix for one client, every run small (N <= 8, under a
# minute each), one at a time. Stops if a run's window was not on screen
# (another Space): its frame numbers would not be valid.
#
#   ./bench.sh swift|gpui|tauri|slint [tag]
#   VORN_SPIKE_WEBVIEW_120=1 ./bench.sh tauri tauri120
set -eu
client=$1
tag=${2:-$1}
root=$(cd "$(dirname "$0")" && pwd)
h="$root/target/release/spike-harness"

run() {
	name=$1
	shift
	timeout 150 "$h" run --client "$client" --name "$name" "$@" 2>&1 | tail -1
	if ! grep -q '"on_screen": true' "$root/results/raw/$name.json"; then
		echo "$name: window not on screen; stopping" >&2
		exit 3
	fi
}

run "$tag-latency-1" --mode latency --panes 1
run "$tag-latency-8-yes" --mode latency --panes 8 --producer yes
run "$tag-idle-1" --mode idle --panes 1
run "$tag-idle-8" --mode idle --panes 8
for p in yes buildlog rec:vim; do
	run "$tag-load-8-$(echo "$p" | tr : _)" --mode load --panes 8 --producer "$p"
done
for i in 1 2 3 4 5; do
	run "$tag-start-1-r$i" --mode start --panes 1
done
