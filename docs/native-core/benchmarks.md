# Native core: benchmark baselines

The numbers the native-core work packages are judged against. `yarn bench`
measures today's JavaScript path; WP2 to WP7 each name the row below that has to
move, and WP9 decides the shell with these in hand.

## Running it

```sh
yarn bench                    # three runs of every suite, compared to this platform's baseline
yarn bench --only=git,flush   # a subset
yarn bench --quick            # one short run: a smoke check, not a number
yarn bench --save             # record this machine as the baseline and rewrite the table below
yarn bench --strict           # exit non-zero if any metric spreads more than 10% across runs
yarn bench --doc              # rewrite the tables below from bench/baselines/*.json
```

Every suite runs in a process of its own, as the `*.process.test.ts`
measurements do, and the runs are interleaved so a machine that slows down part
way spreads its drift over every suite. Each reported value is the median of the
three runs. Within a run, each suite reduces its own repetitions: the CPU suites
(output analysis, screen model, flush, git) keep the fastest repetition, the one
least disturbed by the rest of the machine; the event-loop suite pools every
round into one histogram before taking its percentiles; the renderer suite
pools frame intervals across rounds. The spread column is how far the furthest
run sits from the median, as a share of it; WP0's acceptance bar is 10%.

The renderer suite drives Chromium through `playwright-core`, which does not
download a browser. It uses Playwright's own Chromium when one is installed
(`npx playwright-core install chromium`), then an installed Google Chrome, and
`VORN_BENCH_CHROMIUM=<path>` picks one explicitly.

Results from the last run are written to `bench/results/latest.json`
(untracked). Baselines are per platform in `bench/baselines/<platform>-<arch>.json`;
compare numbers from one machine only.

## What each suite measures

| Suite             | Hotspot         | What is timed                                                                                                                                                     | Moves with                                                                  |
| ----------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `output-analysis` | 1               | `PtyManager.appendOutput` on every raw read, and `stripAnsi` and the status regexes on their own, in ms per MB                                                    | WP3 (spinner row at least 20x lower)                                        |
| `screen-model`    | 2               | The headless xterm parse behind every PTY, to the drain, in ms per MB; one 200x50 serialize                                                                       | WP2                                                                         |
| `flush`           | 7               | `flushBuffer` end to end: client frame, scrollback, screen model, history frame, bell scan, at 1 and 8 clients                                                    | WP2, WP4                                                                    |
| `git`             | 3               | Wall-clock per synchronous git call on a repository with a 460 KB working-tree diff, which is the event-loop stall                                                | WP5                                                                         |
| `event-loop`      | all server ones | p99 and max of a hand-rolled 1 ms timer probe while fake ptys stream through the real handlers, plus what reached the client                                      | WP4 (`burst.50MBps.8s.p99` under 2 ms), WP5 (`agents+git` matches `agents`) |
| `renderer`        | 5               | Frame interval and long-task time in Chromium with the real `terminal-registry.ts` and `TerminalHost`'s per-frame loop, at 1, 8 and 32 terminals each fed 32 KB/s | WP7                                                                         |

The transcripts are generated from a fixed seed (`bench/lib/transcripts.ts`):

- `agent`: the process tests' own generator, coloured cursor-addressed lines, one per ~62-byte read.
- `spinner`: 1 MB of an agent TUI redrawing its status line in place, with tool calls in between, in reads of one to sixty frames.
- `bulk`: 2 MB of build log in 4 KB reads.

Where a suite feeds per flush rather than per read, it groups a hundred reads or
64 KB, whichever comes first: the same assumption `measure-history.ts` makes,
capped at the flush size WP4 introduces, so before and after are fed alike.

The server suites call `PtyManager`'s private handlers through a cast instead of
copying them. A fake pty is wired with the same `setupPtyEvents` a spawned one
gets, so the bench exercises the code WP2 to WP5 replace rather than a model of
it. Running it under `VORN_CORE=native` measures the Rust core instead; that
run keeps its own baseline (`<platform>-native.json`) and never touches the
JS one or the tables here. `yarn bench:compare` runs both, taking turns.

## Caveats

- The renderer suite runs in Chromium, not Electron (the same engine, without the
  rest of the app). In a container without a GPU, Chromium falls back to
  SwiftShader and the renderer numbers are CPU-bound; the GPU string is recorded
  with each baseline. Headless Chromium on macOS uses SwiftShader too, so on
  macOS the suite opens a visible Chromium window with Metal instead; leave it
  in front while it runs. `VORN_BENCH_HEADLESS=1` forces headless anywhere.
- Event-loop numbers under saturation reflect the longest single turn; when the
  loop cannot keep up the generator drops owed output rather than queueing it,
  so read `delivered` beside `p99`.
- Event-loop p99 is the noisiest number: under saturation it is decided by a
  handful of long turns, and on the 4-core shared VM the Linux baseline was
  recorded on, one run in three can land 10-20% off the median. That is far
  inside the margin WP4 is judged on (about 55 ms today against a 2 ms target),
  but read a single-run change in it with care.
- A full `yarn bench` takes about 18 minutes: three runs, and the noisier
  suites run three processes per run. Use `--only` while iterating.
- Absolute numbers move with the machine. Compare against the baseline for the
  same platform, recorded on the same machine.

## Baselines

What the two machines say, before the tables:

- **The server misses WP4 by an order of magnitude on both.** Event-loop p99
  under a 50 MB/s burst is 14-19 ms on an M2 Pro and 55-76 ms on the Linux VM,
  against a 2 ms target. The M2 keeps up with the burst (49.6 MB/s delivered);
  the VM delivers about 30 MB/s of it.
- **Git dominates the loop when it runs.** One `getGitDiffFull` on a 460 KB diff
  stalls the loop for 63 ms on the M2 (32 ms on the VM), and eight busy agents
  with that diff every 250 ms go from 6.5 ms p99 to 101 ms on the M2.
- **`appendOutput` is the per-read hotspot**, mostly the status regexes on small
  reads: 125 ms/MB for 62-byte reads on the M2, of which 72 ms is
  `analyzeOutput`. The spinner transcript, WP3's target, is 52 ms/MB.
- **The renderer is not the bottleneck on a real GPU at this feed.** On the M2
  with Metal, the mean frame interval holds at the 120 Hz vsync (8.4-8.6 ms) at
  1, 8 and 32 terminals each streaming 32 KB/s, and long-task time is near
  zero, which makes those long-task figures noise rather than signal. WP7 needs
  a heavier feed to have something to move; the Linux table's renderer rows are
  software GL and say little about the app.
- **Reproducibility.** On the Linux VM 40 of 41 metrics landed within 10%. On the
  M2, 12 did not: one of the three runs was about 20% slower for every git call
  (the machine was busy, not the code), the 32-session burst p99 swung between
  14 and 28 ms, and the near-zero long-task rows are not measurable at all.
  Everything else, including every ms/MB row, held within 10%.

<!-- bench:darwin-arm64:start -->

Apple M2 Pro, 10 cores, 16 GB, Node v22.23.2, git version 2.54.0 (Apple Git-157). Recorded 2026-10-03 at `2fc17d96`, median of 3 runs.

| Metric                                  |      Median | Spread | What                                                                                                   |
| --------------------------------------- | ----------: | -----: | ------------------------------------------------------------------------------------------------------ |
| `output-analysis/appendOutput.agent`    | 124.5 ms/MB |   4.9% | appendOutput per raw chunk, agent transcript                                                           |
| `output-analysis/stripAnsi.agent`       |  5.63 ms/MB |   6.9% | stripAnsi alone, agent                                                                                 |
| `output-analysis/statusRegex.agent`     | 72.46 ms/MB |   5.9% | analyzeOutput alone (status regexes), agent                                                            |
| `output-analysis/appendOutput.spinner`  | 52.33 ms/MB |   7.9% | appendOutput per raw chunk, spinner transcript                                                         |
| `output-analysis/stripAnsi.spinner`     | 35.16 ms/MB |   8.3% | stripAnsi alone, spinner                                                                               |
| `output-analysis/statusRegex.spinner`   | 13.06 ms/MB |   9.1% | analyzeOutput alone (status regexes), spinner                                                          |
| `output-analysis/appendOutput.bulk`     |  3.46 ms/MB |   8.4% | appendOutput per raw chunk, bulk transcript                                                            |
| `output-analysis/stripAnsi.bulk`        |  1.94 ms/MB |   7.7% | stripAnsi alone, bulk                                                                                  |
| `output-analysis/statusRegex.bulk`      |  0.68 ms/MB |   7.4% | analyzeOutput alone (status regexes), bulk                                                             |
| `screen-model/parse.agent`              | 12.16 ms/MB |   7.3% | headless xterm parse to drain, agent, fed per flush                                                    |
| `screen-model/parse.spinner`            | 19.24 ms/MB |   0.2% | headless xterm parse to drain, spinner, fed per flush                                                  |
| `screen-model/parse.bulk`               | 13.27 ms/MB |     5% | headless xterm parse to drain, bulk, fed per flush                                                     |
| `screen-model/serialize.200x50`         |    1.681 ms |   2.6% | serializeScreen of one 200x50 coloured screen (checkpoint cost per session)                            |
| `flush/flush.agent.1client`             |  3.14 ms/MB |   4.8% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), agent, 1 client(s)     |
| `flush/flush.spinner.1client`           |  2.28 ms/MB |   3.1% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), spinner, 1 client(s)   |
| `flush/flush.bulk.1client`              |  0.66 ms/MB |   7.6% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), bulk, 1 client(s)      |
| `flush/flush.agent.8client`             |  3.01 ms/MB |     4% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), agent, 8 client(s)     |
| `flush/flush.spinner.8client`           |  2.27 ms/MB |   3.5% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), spinner, 8 client(s)   |
| `flush/flush.bulk.8client`              |  0.68 ms/MB |   4.4% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), bulk, 8 client(s)      |
| `git/stall.isGitRepo`                   |    12.24 ms |  21.5% | event-loop stall per isGitRepo call (execFileSync)                                                     |
| `git/stall.getGitBranch`                |    12.39 ms |  20.5% | event-loop stall per getGitBranch call (execFileSync)                                                  |
| `git/stall.getGitStatusPorcelain`       |    16.18 ms |  22.4% | event-loop stall per getGitStatusPorcelain call (execFileSync)                                         |
| `git/stall.getGitDiffStat`              |    19.71 ms |  21.4% | event-loop stall per getGitDiffStat call (execFileSync)                                                |
| `git/stall.getGitDiffText`              |    22.47 ms |  19.9% | event-loop stall per getGitDiffText call (execFileSync)                                                |
| `git/stall.getGitDiffFull`              |    62.84 ms |  21.6% | event-loop stall per getGitDiffFull call (execFileSync)                                                |
| `git/stall.listWorktrees`               |    12.45 ms |    25% | event-loop stall per listWorktrees call (execFileSync)                                                 |
| `event-loop/burst.50MBps.1s.p99`        |    13.84 ms |   7.8% | event-loop delay p99, 50 MB/s of build log across 1 session(s)                                         |
| `event-loop/burst.50MBps.1s.delivered`  |  49.64 MB/s |   0.1% | output that reached the client, 50 MB/s of build log across 1 session(s)                               |
| `event-loop/burst.50MBps.8s.p99`        |    14.46 ms |  17.5% | event-loop delay p99, 50 MB/s of build log across 8 session(s)                                         |
| `event-loop/burst.50MBps.8s.delivered`  |  49.72 MB/s |   0.2% | output that reached the client, 50 MB/s of build log across 8 session(s)                               |
| `event-loop/burst.50MBps.32s.p99`       |    19.17 ms |    48% | event-loop delay p99, 50 MB/s of build log across 32 session(s)                                        |
| `event-loop/burst.50MBps.32s.delivered` |  48.86 MB/s |   1.4% | output that reached the client, 50 MB/s of build log across 32 session(s)                              |
| `event-loop/agents.p99`                 |      6.5 ms |  12.9% | event-loop delay p99, 8 agent TUIs at 1 MB/s each                                                      |
| `event-loop/agents.delivered`           |   8.64 MB/s |   0.1% | output that reached the client, 8 agent TUIs at 1 MB/s each                                            |
| `event-loop/agents+git.p99`             |   100.66 ms |   2.2% | event-loop delay p99, 8 agent TUIs at 1 MB/s each, getGitDiffFull (460 KB diff) every 250 ms           |
| `event-loop/agents+git.delivered`       |   6.09 MB/s |     1% | output that reached the client, 8 agent TUIs at 1 MB/s each, getGitDiffFull (460 KB diff) every 250 ms |
| `renderer/frame.mean.1t`                |     8.46 ms |   0.2% | mean frame interval, 1 terminal(s) streaming                                                           |
| `renderer/longtask.1t`                  |   14.5 ms/s |  43.7% | main-thread long-task time per second, 1 terminal(s) streaming                                         |
| `renderer/frame.mean.8t`                |     8.44 ms |   0.4% | mean frame interval, 8 terminal(s) streaming                                                           |
| `renderer/longtask.8t`                  |   8.58 ms/s | 107.8% | main-thread long-task time per second, 8 terminal(s) streaming                                         |
| `renderer/frame.mean.32t`               |     8.63 ms |   7.6% | mean frame interval, 32 terminal(s) streaming                                                          |
| `renderer/longtask.32t`                 |      0 ms/s |   100% | main-thread long-task time per second, 32 terminal(s) streaming                                        |

Renderer GPU: ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version).

<!-- bench:darwin-arm64:end -->

<!-- bench:linux-x64:start -->

Intel(R) Xeon(R) Processor @ 2.10GHz, 4 cores, 16 GB, Node v22.22.0, git version 2.43.0. Recorded 2026-10-03 at `4db8e9a`, median of 3 runs.

| Metric                                  |       Median | Spread | What                                                                                                   |
| --------------------------------------- | -----------: | -----: | ------------------------------------------------------------------------------------------------------ |
| `output-analysis/appendOutput.agent`    | 190.62 ms/MB |   9.4% | appendOutput per raw chunk, agent transcript                                                           |
| `output-analysis/stripAnsi.agent`       |    8.3 ms/MB |     7% | stripAnsi alone, agent                                                                                 |
| `output-analysis/statusRegex.agent`     |  99.59 ms/MB |   1.3% | analyzeOutput alone (status regexes), agent                                                            |
| `output-analysis/appendOutput.spinner`  |  73.55 ms/MB |   3.6% | appendOutput per raw chunk, spinner transcript                                                         |
| `output-analysis/stripAnsi.spinner`     |   49.7 ms/MB |   8.8% | stripAnsi alone, spinner                                                                               |
| `output-analysis/statusRegex.spinner`   |   16.4 ms/MB |   4.8% | analyzeOutput alone (status regexes), spinner                                                          |
| `output-analysis/appendOutput.bulk`     |   5.47 ms/MB |   3.8% | appendOutput per raw chunk, bulk transcript                                                            |
| `output-analysis/stripAnsi.bulk`        |    2.9 ms/MB |   6.6% | stripAnsi alone, bulk                                                                                  |
| `output-analysis/statusRegex.bulk`      |   1.02 ms/MB |     2% | analyzeOutput alone (status regexes), bulk                                                             |
| `screen-model/parse.agent`              |  17.87 ms/MB |   2.2% | headless xterm parse to drain, agent, fed per flush                                                    |
| `screen-model/parse.spinner`            |  30.18 ms/MB |   4.1% | headless xterm parse to drain, spinner, fed per flush                                                  |
| `screen-model/parse.bulk`               |  20.63 ms/MB |   2.1% | headless xterm parse to drain, bulk, fed per flush                                                     |
| `screen-model/serialize.200x50`         |     1.807 ms |   0.7% | serializeScreen of one 200x50 coloured screen (checkpoint cost per session)                            |
| `flush/flush.agent.1client`             |  11.47 ms/MB |   3.1% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), agent, 1 client(s)     |
| `flush/flush.spinner.1client`           |   7.57 ms/MB |   5.9% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), spinner, 1 client(s)   |
| `flush/flush.bulk.1client`              |   3.61 ms/MB |   0.6% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), bulk, 1 client(s)      |
| `flush/flush.agent.8client`             |  10.61 ms/MB |   6.8% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), agent, 8 client(s)     |
| `flush/flush.spinner.8client`           |   7.48 ms/MB |   4.3% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), spinner, 8 client(s)   |
| `flush/flush.bulk.8client`              |    3.6 ms/MB |   5.3% | flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), bulk, 8 client(s)      |
| `git/stall.isGitRepo`                   |      4.12 ms |   3.2% | event-loop stall per isGitRepo call (execFileSync)                                                     |
| `git/stall.getGitBranch`                |      4.15 ms |   4.1% | event-loop stall per getGitBranch call (execFileSync)                                                  |
| `git/stall.getGitStatusPorcelain`       |      6.27 ms |   3.7% | event-loop stall per getGitStatusPorcelain call (execFileSync)                                         |
| `git/stall.getGitDiffStat`              |      9.24 ms |   3.8% | event-loop stall per getGitDiffStat call (execFileSync)                                                |
| `git/stall.getGitDiffText`              |     11.65 ms |   3.3% | event-loop stall per getGitDiffText call (execFileSync)                                                |
| `git/stall.getGitDiffFull`              |     32.04 ms |   0.6% | event-loop stall per getGitDiffFull call (execFileSync)                                                |
| `git/stall.listWorktrees`               |      4.15 ms |   9.4% | event-loop stall per listWorktrees call (execFileSync)                                                 |
| `event-loop/burst.50MBps.1s.p99`        |     59.08 ms |   4.9% | event-loop delay p99, 50 MB/s of build log across 1 session(s)                                         |
| `event-loop/burst.50MBps.1s.delivered`  |   33.03 MB/s |   3.8% | output that reached the client, 50 MB/s of build log across 1 session(s)                               |
| `event-loop/burst.50MBps.8s.p99`        |     60.36 ms |   3.7% | event-loop delay p99, 50 MB/s of build log across 8 session(s)                                         |
| `event-loop/burst.50MBps.8s.delivered`  |   32.01 MB/s |   1.5% | output that reached the client, 50 MB/s of build log across 8 session(s)                               |
| `event-loop/burst.50MBps.32s.p99`       |     75.76 ms |   1.4% | event-loop delay p99, 50 MB/s of build log across 32 session(s)                                        |
| `event-loop/burst.50MBps.32s.delivered` |   29.02 MB/s |   3.9% | output that reached the client, 50 MB/s of build log across 32 session(s)                              |
| `event-loop/agents.p99`                 |      38.7 ms |   5.8% | event-loop delay p99, 8 agent TUIs at 1 MB/s each                                                      |
| `event-loop/agents.delivered`           |    7.67 MB/s |     3% | output that reached the client, 8 agent TUIs at 1 MB/s each                                            |
| `event-loop/agents+git.p99`             |    111.61 ms |  13.7% | event-loop delay p99, 8 agent TUIs at 1 MB/s each, getGitDiffFull (460 KB diff) every 250 ms           |
| `event-loop/agents+git.delivered`       |    5.31 MB/s |   7.9% | output that reached the client, 8 agent TUIs at 1 MB/s each, getGitDiffFull (460 KB diff) every 250 ms |
| `renderer/frame.mean.1t`                |     37.88 ms |   4.1% | mean frame interval, 1 terminal(s) streaming                                                           |
| `renderer/longtask.1t`                  |  579.75 ms/s |   5.3% | main-thread long-task time per second, 1 terminal(s) streaming                                         |
| `renderer/frame.mean.8t`                |     82.42 ms |   7.3% | mean frame interval, 8 terminal(s) streaming                                                           |
| `renderer/longtask.8t`                  |     845 ms/s |   1.7% | main-thread long-task time per second, 8 terminal(s) streaming                                         |
| `renderer/frame.mean.32t`               |     152.5 ms |   7.2% | mean frame interval, 32 terminal(s) streaming                                                          |
| `renderer/longtask.32t`                 |  948.08 ms/s |   1.2% | main-thread long-task time per second, 32 terminal(s) streaming                                        |

Renderer GPU: ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver).

<!-- bench:linux-x64:end -->
