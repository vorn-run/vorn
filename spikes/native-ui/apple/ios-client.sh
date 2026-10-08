#!/bin/sh
# The harness's client for the iOS app: forwards every VORN_SPIKE_* variable
# to the app in a booted simulator (VORN_SPIKE_SIM, a device UDID) and stays
# attached to its console until it exits, so FIRST_FRAME reaches the harness.
# The simulator's processes run on this Mac, so the app reaches vornd's
# socket under /tmp and writes its report where the harness expects it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
sim=${VORN_SPIKE_SIM:?set VORN_SPIKE_SIM to a booted simulator UDID}
xcrun simctl install "$sim" "$here/build/VornSpikeIOS.app"
for v in $(env | grep '^VORN_SPIKE_' | cut -d= -f1); do
  export "SIMCTL_CHILD_$v=$(printenv "$v")"
done
# simctl block-buffers the app's console when its stdout is a pipe; a pty
# makes it line-buffered so FIRST_FRAME arrives on time.
exec script -q /dev/null xcrun simctl launch --console --terminate-running-process "$sim" run.vorn.spike.ios
