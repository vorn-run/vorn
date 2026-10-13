# Native core: benchmark baselines

The numbers the native-core work packages are judged against. `yarn bench`
measures today's JavaScript path; WP2 to WP7 each name the row below that has to
move, and WP9 decides the shell with these in hand.

## Running it

```sh
yarn bench                    # three runs of every suite, compared to this platform's baseline
yarn bench --only=renderer    # a subset
yarn bench --quick            # one short run: a smoke check, not a number
yarn bench --save             # record this machine as the baseline and rewrite the table below
yarn bench --strict           # exit non-zero if any metric spreads more than 10% across runs
yarn bench --doc              # rewrite the tables below from bench/baselines/*.json
```

Every suite runs in a process of its own, as the `*.process.test.ts`
measurements do, and the runs are interleaved so a machine that slows down part
way spreads its drift over every suite. Each reported value is the median of the
three runs. Within a run, the renderer suite pools frame intervals across
rounds. The spread column is how far the furthest
run sits from the median, as a share of it; WP0's acceptance bar is 10%.

The renderer suite drives Chromium through `playwright-core`, which does not
download a browser. It uses Playwright's own Chromium when one is installed
(`npx playwright-core install chromium`), then an installed Google Chrome, and
`VORN_BENCH_CHROMIUM=<path>` picks one explicitly.

Results from the last run are written to `bench/results/latest.json`
(untracked). Baselines are per platform in `bench/baselines/<platform>-<arch>.json`;
compare numbers from one machine only.

## What each suite measures

| Suite      | Hotspot | What is timed                                                                                                                                                     | Moves with |
| ---------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `renderer` | 5       | Frame interval and long-task time in Chromium with the real `terminal-registry.ts` and `TerminalHost`'s per-frame loop, at 1, 8 and 32 terminals each fed 32 KB/s | WP7        |

The transcripts are generated from a fixed seed (`bench/lib/transcripts.ts`):

- `agent`: the process tests' own generator, coloured cursor-addressed lines, one per ~62-byte read.
- `spinner`: 1 MB of an agent TUI redrawing its status line in place, with tool calls in between, in reads of one to sixty frames.
- `bulk`: 2 MB of build log in 4 KB reads.

The server-side suites (git, output analysis, screen model, flush, event loop,
memory) measured the Node server, which is gone: vornd is the server, and has
its own Rust benches under `packages/core/crates/*/benches`.
The baselines below were recorded before that, and keep only the suites that
still run.

## Many panes: the native UI

`yarn bench:panes` (or `packages/core/scripts/many-pane-bench.sh`) builds the
test daemons and `crates/vorn-app/examples/many_panes.rs` in the ci profile and
runs it. Each case starts its own vornd on a temporary HOME and data directory
(never `~/.vorn`), opens N shells, runs a workload in each, and drives the
native UI offscreen on a 60 Hz clock:

```sh
yarn bench:panes                                   # 8 and 16 panes, every case
yarn bench:panes --panes 32 --cases agent           # one case
yarn bench:panes --size 2560x1440 --out run.json    # a larger window; JSON as well as the table
VORNUI_RENDERER=cpu yarn bench:panes --panes 4      # the CPU raster
```

The workloads are the bench itself re-run in each pane as `many_panes feed`:
an agent TUI at 32 KB/s (the `spinner` transcript, byte for byte the one the
renderer suite above feeds each terminal), a build log at 256 KB/s (`bulk`), and
a full-screen redraw at 60 Hz. One pane on screen echoes keys, for latency.

| Case     | Panes stream                                                          |
| -------- | --------------------------------------------------------------------- |
| `mixed`  | agent TUI, build log and full-screen redraw, in turn                  |
| `agent`  | every pane the agent TUI                                              |
| `hidden` | as `agent`, with the probe's card expanded, so the rest are not drawn |

- **Frame**: one frame's build, layout, paint and GPU submit, including folding in
  what the terminals sent since the last. The budget is 16.7 ms.
- **Input to pixel**: from a key sent to the probe until a frame showing its echo
  has been drawn, so it includes the trip through vornd, the shell's PTY and
  back, and up to one frame of waiting for the clock.
- **CPU**: the app process, and vornd with its session holders, over the run,
  in per cent of one core; per pane is their sum over N. The programs in the
  panes are the workload and are not counted.

The grid shows at most 16 cards (4 by 4, once the grid area is at least 1280
by 1120) and pages past what fits; every pane is attached, as if the person had
paged through them all, and `Shown` counts those drawn.

### Baseline: Apple M2 Pro

Apple M2 Pro, 10 cores, 16 GB, Metal. Recorded with `nice -n 10` on a machine
shared with other builds (load average 8-10), 5 s per case after a 2 s warm-up.
Results: `packages/core/bench/many-panes/darwin-arm64*.json`.

1600x1000 (the renderer suite's viewport; 12 of 16 cards fit):

| Case   | Panes | Shown | Frame p50 | Frame p99 | Over 16.7 ms | Input to pixel p50 |      p99 | CPU app | CPU vornd | CPU per pane |
| ------ | ----: | ----: | --------: | --------: | -----------: | -----------------: | -------: | ------: | --------: | -----------: |
| mixed  |     8 |     8 |   2.78 ms |   3.13 ms |           0% |           13.48 ms | 23.09 ms |   22.6% |     26.6% |         6.2% |
| agent  |     8 |     8 |   2.83 ms |   5.24 ms |           0% |           11.42 ms | 22.97 ms |   23.8% |     23.2% |         5.9% |
| hidden |     8 |     1 |   1.06 ms |   1.28 ms |           0% |            9.67 ms | 20.65 ms |   12.6% |     21.2% |         4.2% |
| mixed  |    16 |    12 |   3.64 ms |   4.42 ms |           0% |           12.27 ms | 23.41 ms |   29.7% |     46.8% |         4.8% |
| agent  |    16 |    12 |   3.57 ms |   3.84 ms |           0% |           14.02 ms | 23.05 ms |   28.6% |     34.2% |         3.9% |
| hidden |    16 |     1 |   1.10 ms |   1.29 ms |           0% |           10.74 ms | 20.29 ms |   13.8% |     33.8% |         3.0% |

2560x1440 (all 16 cards on screen):

| Case   | Panes | Shown | Frame p50 | Frame p99 | Over 16.7 ms | Input to pixel p50 |      p99 | CPU app | CPU vornd | CPU per pane |
| ------ | ----: | ----: | --------: | --------: | -----------: | -----------------: | -------: | ------: | --------: | -----------: |
| mixed  |    16 |    16 |   4.82 ms |   8.30 ms |           0% |           13.41 ms | 22.37 ms |   38.2% |     52.7% |         5.7% |
| agent  |    16 |    16 |   4.53 ms |   6.00 ms |           0% |           13.94 ms | 23.33 ms |   33.6% |     40.3% |         4.6% |
| hidden |    16 |     1 |   1.07 ms |   1.38 ms |           0% |           10.18 ms | 20.68 ms |   12.0% |     35.8% |         3.0% |

What they say:

- Every case holds 60 Hz with room: the worst p99 is 8.3 ms at 16 panes on
  screen. Frame time grows with the panes drawn (about 0.2 ms a pane), not with
  the panes streaming: `hidden` costs the same at 8 and 16.
- Input to pixel sits at one or two frames: the echo makes the trip and waits
  for the next tick of the 60 Hz clock.
- Hidden panes are not drawn, but they are not free either: the app still folds
  their output into a screen every frame (`hidden` keeps about half the app CPU
  of `agent`), and vornd's cost follows the bytes, not what is shown.
- One run under heavier load put a 117 ms p99 on `mixed` at 16 panes; two
  reruns did not repeat it. Rerun before reading a single outlier as a
  regression.

### Comparing with the Electron client

The renderer suite above is the Electron-side measure: the same agent
transcript at 32 KB/s per terminal, in Chromium with the real terminal code, at
1600x1000. It reports the mean frame interval, which on the M2 holds at the
120 Hz vsync (8.44 ms at 8 terminals, 8.63 ms at 32), and main-thread long-task
time, rather than the cost of a frame, so the two do not compare one to one: a
frame interval at vsync says the renderer kept up, and the native frame times
above say by how much. To record it on the same machine:

```sh
yarn bench --only=renderer          # opens a visible Chromium window on macOS; leave it in front
```

For a 16-terminal row, add `{ terminals: 16, shown: 16, key: '16t', ... }` to
`CASES` in `bench/suites/renderer.ts`. The suite needs the repo's
`node_modules` and a Chromium (see Running it above); it was not rerun for the
numbers here.

## Caveats

- The renderer suite runs in Chromium, not Electron (the same engine, without the
  rest of the app). In a container without a GPU, Chromium falls back to
  SwiftShader and the renderer numbers are CPU-bound; the GPU string is recorded
  with each baseline. Headless Chromium on macOS uses SwiftShader too, so on
  macOS the suite opens a visible Chromium window with Metal instead; leave it
  in front while it runs. `VORN_BENCH_HEADLESS=1` forces headless anywhere.
- Use `--only` while iterating.
- Absolute numbers move with the machine. Compare against the baseline for the
  same platform, recorded on the same machine.

## Baselines

What the two machines say, before the tables:

- **The renderer is not the bottleneck on a real GPU at this feed.** On the M2
  with Metal, the mean frame interval holds at the 120 Hz vsync (8.4-8.6 ms) at
  1, 8 and 32 terminals each streaming 32 KB/s, and long-task time is near
  zero, which makes those long-task figures noise rather than signal. The Linux
  table's renderer rows are software GL and say little about the app.

<!-- bench:darwin-arm64:start -->

Apple M2 Pro, 10 cores, 16 GB, Node v22.23.2, git version 2.54.0 (Apple Git-157). Recorded 2026-10-03 at `2fc17d96`, median of 3 runs.

| Metric                    |    Median | Spread | What                                                            |
| ------------------------- | --------: | -----: | --------------------------------------------------------------- |
| `renderer/frame.mean.1t`  |   8.46 ms |   0.2% | mean frame interval, 1 terminal(s) streaming                    |
| `renderer/longtask.1t`    | 14.5 ms/s |  43.7% | main-thread long-task time per second, 1 terminal(s) streaming  |
| `renderer/frame.mean.8t`  |   8.44 ms |   0.4% | mean frame interval, 8 terminal(s) streaming                    |
| `renderer/longtask.8t`    | 8.58 ms/s | 107.8% | main-thread long-task time per second, 8 terminal(s) streaming  |
| `renderer/frame.mean.32t` |   8.63 ms |   7.6% | mean frame interval, 32 terminal(s) streaming                   |
| `renderer/longtask.32t`   |    0 ms/s |   100% | main-thread long-task time per second, 32 terminal(s) streaming |

Renderer GPU: ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version).

<!-- bench:darwin-arm64:end -->

<!-- bench:linux-x64:start -->

Intel(R) Xeon(R) Processor @ 2.10GHz, 4 cores, 16 GB, Node v22.22.0, git version 2.43.0. Recorded 2026-10-03 at `4db8e9a`, median of 3 runs.

| Metric                    |      Median | Spread | What                                                            |
| ------------------------- | ----------: | -----: | --------------------------------------------------------------- |
| `renderer/frame.mean.1t`  |    37.88 ms |   4.1% | mean frame interval, 1 terminal(s) streaming                    |
| `renderer/longtask.1t`    | 579.75 ms/s |   5.3% | main-thread long-task time per second, 1 terminal(s) streaming  |
| `renderer/frame.mean.8t`  |    82.42 ms |   7.3% | mean frame interval, 8 terminal(s) streaming                    |
| `renderer/longtask.8t`    |    845 ms/s |   1.7% | main-thread long-task time per second, 8 terminal(s) streaming  |
| `renderer/frame.mean.32t` |    152.5 ms |   7.2% | mean frame interval, 32 terminal(s) streaming                   |
| `renderer/longtask.32t`   | 948.08 ms/s |   1.2% | main-thread long-task time per second, 32 terminal(s) streaming |

Renderer GPU: ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver).

<!-- bench:linux-x64:end -->
