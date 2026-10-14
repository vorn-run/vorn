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

| Case     | Panes stream                                                             |
| -------- | ------------------------------------------------------------------------ |
| `mixed`  | agent TUI, build log and full-screen redraw, in turn                     |
| `agent`  | every pane the agent TUI                                                 |
| `hidden` | as `agent`, with the probe's card expanded, so the rest are not drawn    |
| `resize` | as `agent`, while the window is dragged to 60% by 70% and back every 2 s |

- **Frame**: one frame's build, layout, paint and GPU submit, including folding in
  what the terminals sent since the last. The budget is 16.7 ms.
- **Input to pixel**: from a key sent to the probe until a frame showing its echo
  has been drawn, so it includes the trip through vornd, the shell's PTY and
  back, and up to one frame of waiting for the clock.
- **CPU**: the app process, and vornd with its session holders, over the run,
  in per cent of one core; per pane is their sum over N. The programs in the
  panes are the workload and are not counted.
- **RSS**: the resident memory of the app process, and of vornd with its session
  holders, at the end of the run.
- **Resize**: in the `resize` case every frame has a new window size and a new
  target to draw into, which stands in for the window's surface being
  reconfigured; the terminals are resized to fit their cards as they go.

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

### 8, 16 and 32 panes, with resize and memory

The same M2 Pro on 2026-10-14 at `ff04c06d`, 1600x1000, Metal, `nice -n 10`,
10 s per case after the 2 s warm-up. Eight rounds over an hour, each tier in its
own process; the table is the median of the eight. The machine was idle but
not quiet: the installed app and endpoint security kept the load average at 1.5
to 5. Results: `packages/core/bench/many-panes/darwin-arm64-2026-10-14.json`
(every round, with a `run` field).

| Case   | Panes | Shown | Frame p50 | Frame p99 | Over 16.7 ms | Input to pixel p50 |      p99 | CPU app | CPU vornd | CPU per pane | RSS app | RSS vornd |
| ------ | ----: | ----: | --------: | --------: | -----------: | -----------------: | -------: | ------: | --------: | -----------: | ------: | --------: |
| mixed  |     8 |     8 |   4.61 ms |   6.96 ms |           0% |           14.20 ms | 24.80 ms |   34.8% |     48.8% |        10.4% |   56 MB |     70 MB |
| agent  |     8 |     8 |   5.18 ms |   7.26 ms |           0% |           14.79 ms | 25.09 ms |   37.8% |     41.4% |         9.9% |   65 MB |     69 MB |
| hidden |     8 |     1 |   3.86 ms |   6.55 ms |           0% |           14.00 ms | 23.78 ms |   33.1% |     50.6% |        10.5% |   55 MB |     70 MB |
| resize |     8 |     8 |   4.55 ms |   7.04 ms |           0% |           14.09 ms | 23.98 ms |   36.9% |     41.5% |         9.8% |   71 MB |     68 MB |
| mixed  |    16 |    12 |   4.78 ms |   6.32 ms |           0% |           14.49 ms | 60.57 ms |   36.9% |     81.3% |         7.4% |   61 MB |    116 MB |
| agent  |    16 |    12 |   5.12 ms |   6.55 ms |           0% |           14.41 ms | 25.43 ms |   38.8% |     65.6% |         6.5% |   72 MB |    108 MB |
| hidden |    16 |     1 |   3.62 ms |   5.84 ms |           0% |           13.49 ms | 24.38 ms |   32.6% |     74.3% |         6.7% |   65 MB |    110 MB |
| resize |    16 |    12 |   4.28 ms |   6.58 ms |           0% |           13.93 ms | 24.31 ms |   37.7% |     71.8% |         6.8% |   81 MB |    107 MB |
| mixed  |    32 |    12 |   3.98 ms |   7.87 ms |           0% |           14.20 ms | 50.62 ms |   39.0% |    134.5% |         5.4% |   66 MB |    210 MB |
| agent  |    32 |    12 |   4.83 ms |   5.94 ms |           0% |           14.43 ms | 24.04 ms |   38.4% |     97.0% |         4.2% |   78 MB |    182 MB |
| hidden |    32 |     1 |   3.42 ms |   5.68 ms |           0% |           13.44 ms | 23.38 ms |   34.7% |    131.2% |         5.2% |   68 MB |    183 MB |
| resize |    32 |    12 |   4.16 ms |   5.88 ms |           0% |           13.93 ms | 24.12 ms |   37.9% |    109.9% |         4.6% |   83 MB |    184 MB |

What they say:

- Every case holds 60 Hz: no frame of the 96 runs went over 16.7 ms more than
  0.2% of the time, the worst single frame was 21 ms, and no probe key timed
  out. Frame time does not grow from 16 to 32 panes, since the grid draws 12
  cards either way.
- Resizing costs nothing extra: a new size and target every frame, with the
  terminals refitted as they go, sits at or below the still `agent` case.
- vornd's CPU follows the bytes: about 41% at 8 panes, 66% at 16 and 97% at 32
  for the agent feed, so CPU per pane falls as panes are added (9.9%, 6.5%,
  4.2%). Its memory grows by about 4.7 MB a pane (69, 108, 182 MB); the app's
  stays at 55 to 85 MB.
- Frame times are about twice the October 13 baseline. The #769 build, rerun
  between these rounds, gives the same numbers, so this is the machine, not
  the code: one quiet round (16 panes) matched the baseline, with `hidden` at
  1.01 ms and `agent` at 2.67 ms.
- `mixed` at 16 and 32 panes has a wide input p99 (34 to 75 ms over the
  rounds) where `mixed` at 8 holds at 24-25 ms. Five interleaved pairs at 16
  panes put the #769 build at 23-24 ms p99 and 70% vornd CPU, and this build at
  25-72 ms and 82%. The cause was the megabyte of history each terminal keeps
  since #777: every checkpoint hashed its fingerprint, history included, with a
  byte-wise FNV-1a twice, about 40 ms a cut on the engine thread. Cuts now hash
  with CRC-32 alone; ten interleaved runs put `mixed` at 16 panes at 24.5 ms
  p99 and 59% vornd CPU, against 56.0 ms and 71% before and 24.5 ms and 56% at
  #769.

No Electron row: this bench links the native app's code and draws it offscreen,
so it cannot drive the Electron client, and the renderer suite that measured
Chromium left the repo with it. A comparable baseline needs the beta.8 app run
from a copy, on its own HOME, `--user-data-dir` and `VORN_DATA_DIR` (so its
vornd is a test one), with `--remote-debugging-port`; the same `many_panes feed`
programs started in N shells over its vornd; frame cost from a DevTools
timeline trace rather than the frame interval; keys sent through the debugging
protocol with the echo read back from the terminal; and CPU and RSS summed
over the Electron processes and vornd.

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

## The Tasks board

`yarn bench:board` (or `packages/core/scripts/board-bench.sh`) builds the test
daemons and `crates/vorn-app/examples/board.rs` in the ci profile and runs it.
Each tier starts its own vornd on a temporary HOME and data directory, seeds
that many tasks across the five columns, and drives the board offscreen on a
60 Hz clock: idle frames, a wheel scroll down the first column, and a card
dragged across the board and dropped.

```sh
yarn bench:board                                          # 100, 1,000 and 10,000 tasks, window
yarn bench:board --surface cells                          # the terminal UI's cell grid
yarn bench:board --tiers 10000 --reps 1 --out run.json    # one tier; JSON as well as the table
```

- **Frame**: build, layout and paint of one frame (and GPU submit on a window).
  The budget is 16.7 ms.
- **Board** and **Footprint**: the process's physical footprint once the board
  is shown, less the footprint before it, and at the end of the run.

Columns build only the cards in or near view; the rest are spacers of their
measured or guessed height, so a frame costs about the same at any size.
`packages/core/bench/board` keeps the JSON of the runs below.

### Before and after: Apple M2 Pro

Window 1440x900@2 (median of 3 runs after, 3 before at 100 and 1,000 and 1 at
10,000); cells 180x32, 1 run. Frame p99 over the scroll and drag scripts.

| Surface | Tasks  | Scroll p99 before | Drag p99 before | Scroll p99 after | Drag p99 after | Max after | Footprint before | Footprint after |
| ------- | -----: | ----------------: | --------------: | ---------------: | -------------: | --------: | ---------------: | --------------: |
| Window  | 100    | 33.9 ms           | 37.6 ms         | 7.8 ms           | 8.3 ms         | 9.5 ms    | 93 MB            | 84 MB           |
| Window  | 1,000  | 326 ms            | 453 ms          | 8.9 ms           | 9.0 ms         | 10.6 ms   | 886 MB           | 93 MB           |
| Window  | 10,000 | 3,398 ms          | 3,811 ms        | 9.5 ms           | 8.2 ms         | 14.0 ms   | 3,588 MB         | 123 MB          |
| Cells   | 100    | 48.1 ms           | 49.6 ms         | 9.1 ms           | 8.6 ms         | 9.3 ms    | 17 MB            | 13 MB           |
| Cells   | 1,000  | 474 ms            | 469 ms          | 9.5 ms           | 12.5 ms        | 12.6 ms   | 648 MB           | 22 MB           |
| Cells   | 10,000 | 5,030 ms          | 4,765 ms        | 11.0 ms          | 9.7 ms         | 15.5 ms   | 885 MB           | 52 MB           |

Before, every frame missed the budget at every tier; after, none does. Showing
the board at 10,000 tasks takes 102 ms on a window (7.0 s before) and 98 ms on
cells (4.9 s before).

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
