#!/bin/sh
# Fetches GPUI at the pinned commit (shallow, sparse) into ../.deps/zed and
# applies the spike's test-support hooks.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
zed="$here/../.deps/zed"
REV=f16f9652ec57bf806e65b2a0d51bb92a63644914
if [ ! -d "$zed/.git" ]; then
  mkdir -p "$here/../.deps"
  git init -q "$zed"
  git -C "$zed" remote add origin https://github.com/zed-industries/zed.git
  git -C "$zed" sparse-checkout set crates script
  git -C "$zed" fetch -q --depth 1 origin "$REV"
  git -C "$zed" checkout -q FETCH_HEAD
fi
"${PYTHON:-python3}" "$here/patch_zed.py" "$zed"
