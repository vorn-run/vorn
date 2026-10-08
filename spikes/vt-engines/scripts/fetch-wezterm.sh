#!/bin/sh
# wezterm-term is not published on crates.io; fetch its upstream at a pinned commit, without submodules.
set -eu
REV=372548295b0b25c9a5400f0ea56f0e89f47e0524
# Outside the spike's directory: wezterm's crates inherit from their own workspace root, which cargo would otherwise take to be ours.
DIR="$(cd "$(dirname "$0")/../.." && pwd)/.vt-vendor/wezterm"
[ -d "$DIR/.git" ] || git clone -q --filter=blob:none --no-checkout https://github.com/wezterm/wezterm "$DIR"
git -C "$DIR" fetch -q origin "$REV" 2>/dev/null || true
git -C "$DIR" checkout -q "$REV"
