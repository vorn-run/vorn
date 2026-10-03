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
- Absolute numbers move with the machine. Compare against the baseline for the
  same platform, recorded on the same machine.

## Baselines

<!-- bench:linux-x64:start -->
<!-- bench:linux-x64:end -->
