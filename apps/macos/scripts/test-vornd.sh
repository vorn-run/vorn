#!/bin/sh
# Runs a throwaway vornd on a temp data directory, for the preview host and tests.
# usage: scripts/test-vornd.sh <data-dir> [port]   (cargo build --profile ci -p vornd -p vorn-sessiond first)
set -eu
dir=${1:?give a data directory, never ~/.vorn}
port=${2:-50991}
case "$(cd "$(dirname "$dir")" 2>/dev/null && pwd)/$(basename "$dir")" in
"$HOME/.vorn" | "$HOME/.vorn/") echo "refusing the real data directory" >&2; exit 2 ;;
esac
core="$(cd "$(dirname "$0")/../../../packages/core" && pwd)"
mkdir -p "$dir/home"
HOME="$(cd "$dir" && pwd)/home" exec "$core/target/ci/vornd" --data-dir "$dir" --port "$port" --host 127.0.0.1 \
  --sessiond "$core/target/ci/vorn-sessiond" --log-file "$dir/vornd.log"
