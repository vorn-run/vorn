# Ghostty core vs JS: spike results

Spike branch `claude/ghostty-vs-js-spike-j19qh3`, on top of the WP0 bench (#639) and the WP1 core scaffold (#638). `VORN_CORE=native` routes the two measured hotspots through `packages/core`:

- **Screen model**: `terminal-screen.ts` feeds a libghostty-vt `Terminal` instead of a headless xterm, and serializes with Ghostty's VT formatter.
- **Output analysis**: `appendOutput`'s ANSI strip, line ring, bracketed-paste and status patterns run in one Rust pass per raw chunk (`packages/core/src/analysis.rs`).

Everything else (flush fan-out, scrollback, history, idle timers) is the same code in both modes. `yarn bench:compare --runs=3` runs every suite under each core in separate processes. Numbers include the napi crossing on every chunk and flush, with the bench's real chunk sizes (62 B keystroke-sized reads, 1-60 frame reads, 4 KB pages).

## Verdict

The native core is faster on everything it replaces, on both machines.

- **End to end, one megabyte of terminal output costs the server 4-6x less CPU** (agent 6.4x, spinner 5.8x, build log 4.3x on the M2 Pro).
- **Event-loop p99 drops 3x under a 50 MB/s burst** (15.5 to 5.6 ms on the M2 Pro) and **16x with git running beside 8 agents** (104 to 6.4 ms).
- **Memory per session drops 2.5x in RSS and 14x on the V8 heap.**
- **Checkpoint serialize is 40-46x faster** (1.6 ms to 0.04 ms per 200x50 screen).

Two of the plan's acceptance bars are **not met yet** by this spike, and neither is the core's fault:

- WP3 asks for 20x on `appendOutput.spinner`; this is 11x. The Rust analysis itself runs at about 2.5 ms/MB and a napi call per chunk adds 1-3 ms/MB. Most of what is left is JS bookkeeping per chunk that both paths share, mainly re-arming the idle `setTimeout` on every read (measured at about 7 ms/MB on the agent transcript). Replacing that with a timestamp check is a small JS change.
- WP4 asks for event-loop p99 under 2 ms at 50 MB/s; this is 5.6 ms on the M2 Pro. WP4's own work (64 KB flush cap, backpressure, history off the loop) has not been done here.

`flush` reads as slower under native only because the Ghostty parse now happens inside the flush, synchronously. Under JS the xterm parse is queued and runs in later turns, which `screen-model/parse` times separately. The "server total per MB" rows add both up for a fair comparison.

## Caveats

- The native analysis is closer to a real terminal than the JS one: escape sequences split across reads are still stripped, `\r` and erase-in-line apply across reads, so a redrawn status line stays one line. Status detection tests pass unchanged with `VORN_CORE=native`.
- With `VORN_CORE=native`, 34 of 37 `terminal-screen` tests pass. The three failures are known WP2 gaps: labels restored from a checkpoint, the title length bound, and percent-decoding an OSC 7 path.
- Ghostty is built for the baseline CPU (shippable). On the Linux sandbox its raw parse is 150-270 MB/s, about 10% faster with `-Dcpu=native`.
- Building on macOS 27.2 fails out of the box: the SDK `.tbd` stubs list only `arm64e`, so Zig 0.15.2 cannot link. The Mac run used a scratch copy of the 26.5 SDK with `arm64-macos` added and an `xcrun` shim, without changing the system.
- Linux sandbox native numbers spread up to 28% across runs (shared VM); the M2 Pro run is the one to quote.

## Apple M2 Pro

10 cores, 16 GB, macOS 27.2, Node v22.23.2, on AC power with other apps open. Recorded 2026-10-03 at `fd5c7e0`, median of 3 runs per core.

| Metric                                 |             JS |         Native |               Native vs JS |
| -------------------------------------- | -------------: | -------------: | -------------------------: |
| **server total per MB, agent**         |   139.29 ms/MB |    21.85 ms/MB |                   **6.4x** |
| **server total per MB, spinner**       |    73.66 ms/MB |    12.74 ms/MB |                   **5.8x** |
| **server total per MB, bulk**          |    17.20 ms/MB |     4.03 ms/MB |                   **4.3x** |
| `output-analysis/appendOutput.agent`   |   124.87 ms/MB |    14.94 ms/MB |                   **8.4x** |
| `output-analysis/appendOutput.spinner` |    52.35 ms/MB |     4.56 ms/MB |                    **11x** |
| `output-analysis/appendOutput.bulk`    |     3.46 ms/MB |     1.25 ms/MB |                   **2.8x** |
| `output-analysis/batched.agent`        |              – |     8.47 ms/MB |                          – |
| `output-analysis/napiFloor.agent`      |              – |     2.55 ms/MB |                          – |
| `output-analysis/batched.spinner`      |              – |     3.53 ms/MB |                          – |
| `output-analysis/napiFloor.spinner`    |              – |     1.03 ms/MB |                          – |
| `output-analysis/batched.bulk`         |              – |     1.14 ms/MB |                          – |
| `output-analysis/napiFloor.bulk`       |              – |     0.11 ms/MB |                          – |
| `screen-model/parse.agent`             |    11.43 ms/MB |     3.94 ms/MB |                   **2.9x** |
| `screen-model/parse.spinner`           |    19.04 ms/MB |     5.79 ms/MB |                   **3.3x** |
| `screen-model/parse.bulk`              |    13.07 ms/MB |      1.9 ms/MB |                   **6.9x** |
| `screen-model/serialize.200x50`        |       1.646 ms |       0.036 ms |                    **46x** |
| `flush/flush.agent.1client`            |     2.99 ms/MB |     6.91 ms/MB | 0.43x (includes the parse) |
| `flush/flush.spinner.1client`          |     2.27 ms/MB |     8.18 ms/MB | 0.28x (includes the parse) |
| `flush/flush.bulk.1client`             |     0.67 ms/MB |     2.78 ms/MB | 0.24x (includes the parse) |
| `event-loop/burst.50MBps.1s.p99`       |        14.2 ms |        4.92 ms |                   **2.9x** |
| `event-loop/burst.50MBps.8s.p99`       |       15.47 ms |        5.62 ms |                   **2.8x** |
| `event-loop/burst.50MBps.32s.p99`      |        19.1 ms |         5.6 ms |                   **3.4x** |
| `event-loop/burst.50MBps.*.delivered`  | 48.9–49.7 MB/s | 49.4–49.9 MB/s |                       1.0x |
| `event-loop/agents.p99`                |        6.96 ms |        4.31 ms |                   **1.6x** |
| `event-loop/agents+git.p99`            |      104.14 ms |        6.42 ms |                    **16x** |
| `event-loop/agents+git.delivered`      |         6 MB/s |      5.72 MB/s |                      0.95x |
| `memory/rss.perSession`                |       1.248 MB |       0.505 MB |                   **2.5x** |
| `memory/heap.perSession`               |       0.821 MB |       0.057 MB |                    **14x** |

## Linux sandbox

Intel(R) Xeon(R) Processor @ 2.10GHz, 4 cores, 16 GB, Node v22.22.0. Recorded 2026-10-03 at `fd5c7e0`, median of 3 run(s) per core.

| Metric                                  |           JS |      Native |   Native vs JS |
| --------------------------------------- | -----------: | ----------: | -------------: |
| **server total per MB, agent**          | 215.86 ms/MB | 34.42 ms/MB |       **6.3x** |
| **server total per MB, spinner**        | 107.02 ms/MB | 23.84 ms/MB |       **4.5x** |
| **server total per MB, bulk**           |  28.73 ms/MB |  8.92 ms/MB |       **3.2x** |
| `output-analysis/appendOutput.agent`    | 187.13 ms/MB | 18.84 ms/MB |       **9.9x** |
| `output-analysis/stripAnsi.agent`       |    8.1 ms/MB |           – |              – |
| `output-analysis/statusRegex.agent`     |   95.3 ms/MB |           – |              – |
| `output-analysis/appendOutput.spinner`  |  71.38 ms/MB |  6.78 ms/MB |        **11x** |
| `output-analysis/stripAnsi.spinner`     |  45.87 ms/MB |           – |              – |
| `output-analysis/statusRegex.spinner`   |  15.79 ms/MB |           – |              – |
| `output-analysis/appendOutput.bulk`     |   5.29 ms/MB |  2.19 ms/MB |       **2.4x** |
| `output-analysis/stripAnsi.bulk`        |   2.82 ms/MB |           – |              – |
| `output-analysis/statusRegex.bulk`      |   0.96 ms/MB |           – |              – |
| `output-analysis/batched.agent`         |            – |  7.63 ms/MB |              – |
| `output-analysis/napiFloor.agent`       |            – |  3.03 ms/MB |              – |
| `output-analysis/batched.spinner`       |            – |     5 ms/MB |              – |
| `output-analysis/napiFloor.spinner`     |            – |  1.93 ms/MB |              – |
| `output-analysis/batched.bulk`          |            – |  1.99 ms/MB |              – |
| `output-analysis/napiFloor.bulk`        |            – |  0.22 ms/MB |              – |
| `screen-model/parse.agent`              |   17.5 ms/MB |  6.36 ms/MB |       **2.8x** |
| `screen-model/parse.spinner`            |  27.96 ms/MB |  7.57 ms/MB |       **3.7x** |
| `screen-model/parse.bulk`               |  20.03 ms/MB |  3.08 ms/MB |       **6.5x** |
| `screen-model/serialize.200x50`         |     1.757 ms |    0.043 ms |        **41x** |
| `flush/flush.agent.1client`             |  11.23 ms/MB | 15.58 ms/MB | 0.72x (slower) |
| `flush/flush.spinner.1client`           |   7.68 ms/MB | 17.06 ms/MB | 0.45x (slower) |
| `flush/flush.bulk.1client`              |   3.41 ms/MB |  6.73 ms/MB | 0.51x (slower) |
| `flush/flush.agent.8client`             |  10.94 ms/MB | 16.46 ms/MB | 0.66x (slower) |
| `flush/flush.spinner.8client`           |   7.39 ms/MB | 15.57 ms/MB | 0.47x (slower) |
| `flush/flush.bulk.8client`              |   3.54 ms/MB | 12.08 ms/MB | 0.29x (slower) |
| `event-loop/burst.50MBps.1s.p99`        |     54.49 ms |     8.79 ms |       **6.2x** |
| `event-loop/burst.50MBps.1s.delivered`  |   34.98 MB/s |  49.85 MB/s |       **1.4x** |
| `event-loop/burst.50MBps.8s.p99`        |     55.51 ms |    10.17 ms |       **5.5x** |
| `event-loop/burst.50MBps.8s.delivered`  |   32.26 MB/s |  49.79 MB/s |       **1.5x** |
| `event-loop/burst.50MBps.32s.p99`       |      73.2 ms |    19.73 ms |       **3.7x** |
| `event-loop/burst.50MBps.32s.delivered` |   31.31 MB/s |  47.99 MB/s |       **1.5x** |
| `event-loop/agents.p99`                 |     39.94 ms |     3.97 ms |        **10x** |
| `event-loop/agents.delivered`           |    7.78 MB/s |   8.64 MB/s |       **1.1x** |
| `event-loop/agents+git.p99`             |    102.83 ms |    12.38 ms |       **8.3x** |
| `event-loop/agents+git.delivered`       |    5.65 MB/s |   7.04 MB/s |       **1.2x** |
| `memory/rss.perSession`                 |      1.32 MB |     0.48 MB |       **2.8x** |
| `memory/heap.perSession`                |     0.818 MB |    0.057 MB |        **14x** |
