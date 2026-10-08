#!/bin/sh
# usage: apple/tools/iosshot.sh <udid> <client> <outname> [--polish]
cd "$(dirname "$0")/../.."
VORN_SPIKE_SIM=$1 TMPDIR=/tmp timeout 90 ./target/release/spike-harness run --client $2 --mode look --settle-ms 14000 --name $3 $4 > target/$3.log 2>&1 &
sleep 13
xcrun simctl io $1 screenshot results/look/$3.png >/dev/null 2>&1
wait
tail -2 target/$3.log
