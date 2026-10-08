#!/usr/bin/env bash
# Runs the scale bench on a throwaway GCP Spot VM in one command: creates
# the VM, builds the release binaries there, runs the tiers, copies the
# results back and appends them to packages/core/bench/scale/RESULTS.md.
# The VM and its disk are deleted on the way out, however the run ends.
#
#   scripts/bench-gcp.sh [TIER...]          (default: 100 1000 10000)
#
# Env: PROJECT (vorn-bench-1008k), ZONE (us-central1-a),
#      MACHINE (n2-standard-16), MAX_RUN (3h).
set -euo pipefail

cd "$(dirname "$0")/.."
root=$(pwd)
project=${PROJECT:-vorn-bench-1008k}
zone=${ZONE:-us-central1-a}
machine=${MACHINE:-n2-standard-16}
max_run=${MAX_RUN:-3h}
tiers=("$@")
vm="vorn-scale-bench-$(date +%Y%m%d-%H%M%S)"
commit=$(git rev-parse --short HEAD)
date=$(date -u +%Y-%m-%d)
dest="packages/core/bench/scale/results/$date-$commit"
gc=(gcloud --quiet --project "$project")

if ! git diff --quiet HEAD -- packages/core; then
  echo "packages/core has uncommitted changes; the VM builds HEAD" >&2
  exit 1
fi

cleanup() {
  local code=$?
  trap - EXIT INT TERM
  echo "deleting $vm"
  "${gc[@]}" compute instances delete "$vm" --zone "$zone" --delete-disks=all >/dev/null 2>&1 || true
  # A disk left behind by a failed create or a detach.
  for d in $("${gc[@]}" compute disks list --filter="name~^$vm" --format='value(name)' 2>/dev/null); do
    "${gc[@]}" compute disks delete "$d" --zone "$zone" >/dev/null 2>&1 || true
  done
  "${gc[@]}" compute instances list --filter="name=$vm" --format='value(name)'
  exit "$code"
}
trap cleanup EXIT INT TERM

ssh_vm() { "${gc[@]}" compute ssh "$vm" --zone "$zone" -- "$@"; }

# Waits until ssh answers with a boot other than `$1`.
wait_boot() {
  for _ in $(seq 1 60); do
    now=$(ssh_vm cat /proc/sys/kernel/random/boot_id 2>/dev/null) && [ -n "$now" ] && [ "$now" != "$1" ] && return
    sleep 10
  done
  echo "the VM did not come up" >&2
  return 1
}

echo "creating $vm ($machine, Spot, $zone)"
"${gc[@]}" compute instances create "$vm" \
  --zone "$zone" \
  --machine-type "$machine" \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration="$max_run" \
  --image-family=ubuntu-2404-lts-amd64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=50GB \
  --boot-disk-type=pd-balanced \
  --labels=purpose=vorn-scale-bench >/dev/null

wait_boot none

echo "copying packages/core at $commit"
git archive --format=tar --prefix=vorn/ HEAD packages/core | ssh_vm 'tar -x -C "$HOME"'

echo "setting up"
ssh_vm 'bash "$HOME/vorn/packages/core/bench/scale/setup-vm.sh"'
boot=$(ssh_vm cat /proc/sys/kernel/random/boot_id)
ssh_vm 'sudo systemctl reboot' || true
wait_boot "$boot"

# Detached on the VM, so a dropped ssh connection does not end the run.
echo "building and running tiers ${tiers[*]:-100 1000 10000}"
ssh_vm "mkdir -p \"\$HOME/results\" && setsid nohup bash -c 'bash \"\$HOME/vorn/packages/core/bench/scale/run-tiers.sh\" \"\$HOME/results\" ${tiers[*]:-}; touch \"\$HOME/results/done\"' >\"\$HOME/run.log\" 2>&1 </dev/null &"
misses=0
while :; do
  sleep 60
  state=$(ssh_vm 'tail -n 1 "$HOME/run.log"; [ -f "$HOME/results/done" ] && echo DONE' 2>/dev/null) || state=
  [ -n "$state" ] && misses=0 || misses=$((misses + 1))
  # Ten silent minutes: the Spot VM was preempted or hit its run limit.
  [ "$misses" -lt 10 ] || { echo "lost the VM" >&2; exit 1; }
  echo "$state" | head -n 1
  case $state in *DONE) break ;; esac
done
ssh_vm 'cp "$HOME/run.log" "$HOME/results/run.log"; rm -f "$HOME/results/done"'

mkdir -p "$dest"
ssh_vm 'tar -c -C "$HOME/results" .' | tar -x -C "$dest"

if ls "$dest"/tier-*.json >/dev/null 2>&1; then
  {
    echo
    ssh_vm "cd \"\$HOME/results\" && \"\$HOME/vorn/packages/core/target/release/vorn-scale-bench\" report \
      --date $date --commit $commit --machine $machine \$(ls tier-*.json | sort -t- -k2 -n)"
  } >>"$root/packages/core/bench/scale/RESULTS.md"
  echo "appended to packages/core/bench/scale/RESULTS.md; raw results in $dest"
else
  echo "no tier wrote results; see $dest" >&2
fi
