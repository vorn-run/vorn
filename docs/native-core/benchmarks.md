# Native core: benchmark baselines

The numbers the native-core work packages are judged against. `yarn bench`
measures today's JavaScript path; WP2 to WP7 each name the row below that has to
move, and WP9 decides the shell with these in hand.

## Running it

```sh
yarn bench                    # three runs of every suite, compared to this platform's baseline
yarn bench --only=git         # a subset
yarn bench --quick            # one short run: a smoke check, not a number
yarn bench --save             # record this machine as the baseline and rewrite the table below
yarn bench --strict           # exit non-zero if any metric spreads more than 10% across runs
yarn bench --doc              # rewrite the tables below from bench/baselines/*.json
```

Every suite runs in a process of its own, as the `*.process.test.ts`
measurements do, and the runs are interleaved so a machine that slows down part
way spreads its drift over every suite. Each reported value is the median of the
three runs. Within a run, each suite reduces its own repetitions: the git suite
keeps the fastest repetition, the one least disturbed by the rest of the
machine; the renderer suite pools frame intervals across rounds. The spread column is how far the furthest
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
| `git`      | 3       | Wall-clock per synchronous git call on a repository with a 460 KB working-tree diff, which is the event-loop stall                                                | WP5        |
| `renderer` | 5       | Frame interval and long-task time in Chromium with the real `terminal-registry.ts` and `TerminalHost`'s per-frame loop, at 1, 8 and 32 terminals each fed 32 KB/s | WP7        |

The transcripts are generated from a fixed seed (`bench/lib/transcripts.ts`):

- `agent`: the process tests' own generator, coloured cursor-addressed lines, one per ~62-byte read.
- `spinner`: 1 MB of an agent TUI redrawing its status line in place, with tool calls in between, in reads of one to sixty frames.
- `bulk`: 2 MB of build log in 4 KB reads.

The server-side terminal suites (output analysis, screen model, flush, event
loop, memory) measured the Node terminal path, which is gone: terminals run in
vornd, which has its own Rust benches under `packages/core/crates/*/benches`.
The baselines below were recorded before that, and keep only the suites that
still run.

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

- **Git dominates the loop when it runs.** One `getGitDiffFull` on a 460 KB diff
  stalls the loop for 63 ms on the M2 (32 ms on the VM), on the execFileSync
  path these baselines were recorded on.
- **The renderer is not the bottleneck on a real GPU at this feed.** On the M2
  with Metal, the mean frame interval holds at the 120 Hz vsync (8.4-8.6 ms) at
  1, 8 and 32 terminals each streaming 32 KB/s, and long-task time is near
  zero, which makes those long-task figures noise rather than signal. The Linux
  table's renderer rows are software GL and say little about the app.

<!-- bench:darwin-arm64:start -->

Apple M2 Pro, 10 cores, 16 GB, Node v22.23.2, git version 2.54.0 (Apple Git-157). Recorded 2026-10-03 at `2fc17d96`, median of 3 runs.

| Metric                            |    Median | Spread | What                                                            |
| --------------------------------- | --------: | -----: | --------------------------------------------------------------- |
| `git/stall.isGitRepo`             |  12.24 ms |  21.5% | event-loop stall per isGitRepo call (execFileSync)              |
| `git/stall.getGitBranch`          |  12.39 ms |  20.5% | event-loop stall per getGitBranch call (execFileSync)           |
| `git/stall.getGitStatusPorcelain` |  16.18 ms |  22.4% | event-loop stall per getGitStatusPorcelain call (execFileSync)  |
| `git/stall.getGitDiffStat`        |  19.71 ms |  21.4% | event-loop stall per getGitDiffStat call (execFileSync)         |
| `git/stall.getGitDiffText`        |  22.47 ms |  19.9% | event-loop stall per getGitDiffText call (execFileSync)         |
| `git/stall.getGitDiffFull`        |  62.84 ms |  21.6% | event-loop stall per getGitDiffFull call (execFileSync)         |
| `git/stall.listWorktrees`         |  12.45 ms |    25% | event-loop stall per listWorktrees call (execFileSync)          |
| `renderer/frame.mean.1t`          |   8.46 ms |   0.2% | mean frame interval, 1 terminal(s) streaming                    |
| `renderer/longtask.1t`            | 14.5 ms/s |  43.7% | main-thread long-task time per second, 1 terminal(s) streaming  |
| `renderer/frame.mean.8t`          |   8.44 ms |   0.4% | mean frame interval, 8 terminal(s) streaming                    |
| `renderer/longtask.8t`            | 8.58 ms/s | 107.8% | main-thread long-task time per second, 8 terminal(s) streaming  |
| `renderer/frame.mean.32t`         |   8.63 ms |   7.6% | mean frame interval, 32 terminal(s) streaming                   |
| `renderer/longtask.32t`           |    0 ms/s |   100% | main-thread long-task time per second, 32 terminal(s) streaming |

Renderer GPU: ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version).

<!-- bench:darwin-arm64:end -->

<!-- bench:linux-x64:start -->

Intel(R) Xeon(R) Processor @ 2.10GHz, 4 cores, 16 GB, Node v22.22.0, git version 2.43.0. Recorded 2026-10-03 at `4db8e9a`, median of 3 runs.

| Metric                            |      Median | Spread | What                                                            |
| --------------------------------- | ----------: | -----: | --------------------------------------------------------------- |
| `git/stall.isGitRepo`             |     4.12 ms |   3.2% | event-loop stall per isGitRepo call (execFileSync)              |
| `git/stall.getGitBranch`          |     4.15 ms |   4.1% | event-loop stall per getGitBranch call (execFileSync)           |
| `git/stall.getGitStatusPorcelain` |     6.27 ms |   3.7% | event-loop stall per getGitStatusPorcelain call (execFileSync)  |
| `git/stall.getGitDiffStat`        |     9.24 ms |   3.8% | event-loop stall per getGitDiffStat call (execFileSync)         |
| `git/stall.getGitDiffText`        |    11.65 ms |   3.3% | event-loop stall per getGitDiffText call (execFileSync)         |
| `git/stall.getGitDiffFull`        |    32.04 ms |   0.6% | event-loop stall per getGitDiffFull call (execFileSync)         |
| `git/stall.listWorktrees`         |     4.15 ms |   9.4% | event-loop stall per listWorktrees call (execFileSync)          |
| `renderer/frame.mean.1t`          |    37.88 ms |   4.1% | mean frame interval, 1 terminal(s) streaming                    |
| `renderer/longtask.1t`            | 579.75 ms/s |   5.3% | main-thread long-task time per second, 1 terminal(s) streaming  |
| `renderer/frame.mean.8t`          |    82.42 ms |   7.3% | mean frame interval, 8 terminal(s) streaming                    |
| `renderer/longtask.8t`            |    845 ms/s |   1.7% | main-thread long-task time per second, 8 terminal(s) streaming  |
| `renderer/frame.mean.32t`         |    152.5 ms |   7.2% | mean frame interval, 32 terminal(s) streaming                   |
| `renderer/longtask.32t`           | 948.08 ms/s |   1.2% | main-thread long-task time per second, 32 terminal(s) streaming |

Renderer GPU: ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver).

<!-- bench:linux-x64:end -->
