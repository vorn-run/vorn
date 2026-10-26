#!/usr/bin/env bash
# Runs the extension load test (run.sh) on a throwaway GCP Spot VM: creates
# the VM, copies HEAD and the shipped extension packs, builds there, runs,
# and copies the results into OUT_DIR. The VM and its disks are deleted on
# the way out, however the run ends, and the script then checks that the
# project lists no VM of this run.
#
#   packages/core/bench/ext-load/gcp.sh OUT_DIR
#
# Env: PROJECT (required), ZONE (us-central1-c), MACHINE (n2-standard-16),
#      MAX_RUN (3h); REPS, RUN_SECONDS, CAPS, IDLE_MS, IDLE_CAP and IDLE_WAIT
#      go to run.sh.
set -euo pipefail

out=$(mkdir -p "${1:?usage: gcp.sh OUT_DIR}" && cd "$1" && pwd)
cd "$(git rev-parse --show-toplevel)"
project=${PROJECT:?set PROJECT to the GCP project the bench VM runs in}
zone=${ZONE:-us-central1-c}
machine=${MACHINE:-n2-standard-16}
vm="vorn-ext-load-$(date +%Y%m%d-%H%M%S)"
gc=(gcloud --quiet --project "$project")

if ! git diff --quiet HEAD -- packages/core examples; then
  echo "packages/core or examples has uncommitted changes; the VM builds HEAD" >&2
  exit 1
fi
# The packs vornd embeds are fetched here (needs gh), not on the VM.
[ -n "$(ls packages/core/shipped/*.vorn.tgz 2>/dev/null)" ] || node packages/core/scripts/shipped.mjs

cleanup() {
  local code=$?
  trap - EXIT INT TERM HUP
  set +e
  echo "deleting $vm" >&2
  local left=
  for _ in 1 2 3 4 5 6; do
    if [ -n "$("${gc[@]}" compute instances list --filter="name=$vm" --format='value(name)' 2>/dev/null)" ]; then
      "${gc[@]}" compute instances delete "$vm" --zone "$zone" --delete-disks=all >/dev/null 2>&1
    fi
    for d in $("${gc[@]}" compute disks list --filter="name~^$vm" --format='value(name)' 2>/dev/null); do
      "${gc[@]}" compute disks delete "$d" --zone "$zone" >/dev/null 2>&1
    done
    left=$("${gc[@]}" compute instances list --filter="name=$vm" --format='value(name)' 2>/dev/null) || left="$vm?"
    left+=$("${gc[@]}" compute disks list --filter="name~^$vm" --format='value(name)' 2>/dev/null) || left+=" disks?"
    [ -z "$left" ] && break
    sleep 20
  done
  if [ -n "$left" ]; then
    echo "could not delete $vm or its disks; delete them by hand: $left" >&2
    [ "$code" -ne 0 ] || code=1
  fi
  local listed
  listed=$("${gc[@]}" compute instances list --format='value(name)' 2>&1) || listed="(listing failed) $listed"
  if [ -n "$listed" ]; then
    echo "the project still lists instances: $listed" >&2
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM HUP

ssh_vm() { "${gc[@]}" compute ssh "$vm" --zone "$zone" -- "$@"; }

echo "creating $vm ($machine, Spot, $zone)"
"${gc[@]}" compute instances create "$vm" \
  --zone "$zone" \
  --machine-type "$machine" \
  --provisioning-model=SPOT \
  --instance-termination-action=DELETE \
  --max-run-duration="${MAX_RUN:-3h}" \
  --image-family=ubuntu-2404-lts-amd64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=60GB \
  --boot-disk-type=pd-balanced \
  --labels=purpose=vorn-ext-load >/dev/null
for _ in $(seq 1 60); do
  ssh_vm true 2>/dev/null && break
  sleep 10
done

echo "copying HEAD ($(git rev-parse --short HEAD))"
# The workspace builds the example extensions, and vornd compiles packages/mcp/package.json in.
git archive --format=tar --prefix=vorn/ HEAD packages/core packages/mcp/package.json examples/extensions |
  ssh_vm 'tar -x -C "$HOME"'
tar -c -C packages/core shipped | ssh_vm 'tar -x -C "$HOME/vorn/packages/core"'
echo "setting up"
ssh_vm 'bash "$HOME/vorn/packages/core/bench/scale/setup-vm.sh" &&
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq nodejs python3 libwebkit2gtk-4.1-dev libgtk-3-dev >/dev/null'

echo "building and running"
env_vars="REPS=${REPS:-} RUN_SECONDS=${RUN_SECONDS:-} CAPS='${CAPS:-}' IDLE_MS=${IDLE_MS:-} IDLE_CAP=${IDLE_CAP:-} IDLE_WAIT=${IDLE_WAIT:-}"
# Detached on the VM, so a dropped ssh connection does not end the run.
timeout 120 "${gc[@]}" compute ssh "$vm" --zone "$zone" -- \
  "mkdir -p \"\$HOME/results\" && $env_vars setsid nohup bash -c 'bash \"\$HOME/vorn/packages/core/bench/ext-load/run.sh\" \"\$HOME/results\"; echo \$? >\"\$HOME/results/exit\"' >\"\$HOME/run.log\" 2>&1 </dev/null &" </dev/null || true

fetch() { ssh_vm 'tar -c -C "$HOME/results" .' 2>/dev/null | tar -x -C "$out" 2>/dev/null; }
misses=0
while :; do
  sleep 60
  state=$(ssh_vm 'if [ -f "$HOME/results/exit" ]; then echo "DONE $(cat "$HOME/results/exit")"; else echo "running: $(tail -n 1 "$HOME/run.log")"; fi' 2>/dev/null) || state=
  if [ -n "$state" ]; then
    misses=0
    fetch || true
  else
    misses=$((misses + 1))
    if ! "${gc[@]}" compute instances describe "$vm" --zone "$zone" --format='value(status)' 2>/dev/null | grep -q RUNNING; then
      echo "the VM is gone (Spot preemption?); partial results in $out" >&2
      exit 1
    fi
  fi
  [ "$misses" -lt 10 ] || { echo "lost the VM; partial results in $out" >&2; exit 1; }
  case $state in DONE*) break ;; esac
  [ -n "$state" ] && echo "$state"
done
ssh_vm 'cp "$HOME/run.log" "$HOME/results/run.log"'
fetch
echo "results in $out ($state)"
[ "$state" = "DONE 0" ]
