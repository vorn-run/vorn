#!/bin/sh
# Fetches GPUI at the pinned Zed commit (sparse, shallow) and builds the prototype.
set -eu
cd "$(dirname "$0")/.."
REV=cb73ee1d45db3babb14f21efe83f229fcb334d99
if [ ! -d .deps/zed/.git ]; then
  mkdir -p .deps
  git init -q .deps/zed
  git -C .deps/zed remote add origin https://github.com/zed-industries/zed.git
  git -C .deps/zed sparse-checkout set crates script
  git -C .deps/zed fetch -q --depth 1 origin "$REV"
  git -C .deps/zed checkout -q FETCH_HEAD
fi
cd gpui && cargo build --release -j 3
