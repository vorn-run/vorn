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
```

Every suite runs in a process of its own, as the `*.process.test.ts`
measurements do, and the runs are interleaved so a machine that slows down part
way spreads its drift over every suite. Each reported value is the median of the
three runs, and each run's value is itself a median of several repetitions. The
spread column is how far the furthest run sits from that median, as a share of
it; WP0's acceptance bar is 10%.

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
| `event-loop`      | all server ones | `monitorEventLoopDelay` p99 and max while fake ptys stream through the real handlers, plus what reached the client                                                | WP4 (`burst.50MBps.8s.p99` under 2 ms), WP5 (`agents+git` matches `agents`) |
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
it. The `VORN_CORE=js|native` switch from WP1 is not read yet; once the native
path exists, running the bench under each value gives the before and after.

## Caveats

- The renderer suite runs in Chromium, not Electron (the same engine, without the
  rest of the app). In a container without a GPU, Chromium falls back to
  SwiftShader and the renderer numbers are CPU-bound; the GPU string is recorded
  with each baseline. Record a macOS baseline on real hardware before WP7.
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
