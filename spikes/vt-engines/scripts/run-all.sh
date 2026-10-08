#!/usr/bin/env bash
# Runs every measurement, one engine and one process at a time, into results/.
# Build first: cargo build --release -j 3 -p vt-harness
set -euo pipefail
cd "$(dirname "$0")/.."
bin=target/release/vt-harness
out=results
mkdir -p "$out"
engines=(ghostty ghostty-raw alacritty wezterm vt100)
corpora=(build-log vim htop agent seeded)

: >"$out/throughput.jsonl"
for c in "${corpora[@]}"; do
  for e in "${engines[@]}"; do
    "$bin" throughput --engine "$e" --corpus "$c" --mb 8 --runs 15 >>"$out/throughput.jsonl"
  done
done

: >"$out/memory.jsonl"
for c in agent build-log; do
  for e in "${engines[@]}"; do
    "$bin" memory --engine "$e" --corpus "$c" --mb 4 --sessions 32 >>"$out/memory.jsonl"
  done
done

: >"$out/resize.jsonl"
for e in "${engines[@]}"; do
  "$bin" resize --engine "$e" --runs 20 >>"$out/resize.jsonl"
done

: >"$out/compare.jsonl"
for e in ghostty-raw alacritty wezterm vt100; do
  "$bin" compare --engine "$e" >>"$out/compare.jsonl"
done

: >"$out/features.jsonl"
for e in "${engines[@]}"; do
  "$bin" features --engine "$e" >>"$out/features.jsonl"
done
