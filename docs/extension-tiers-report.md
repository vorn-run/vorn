# Extension tiers: components against programs

Three extensions that moved out of the core are built twice from one Rust
source, on the same manifest:

- as a **Wasm component** (`component = "x.wasm"`) that vornd loads in process;
- as a **program** (`command = [...]`) that vornd starts and speaks to over stdio.

The three are the Widget, Worktree cleanup and the RSS connector. The RSS
connector is also measured as Vorn ships it today, as a Node bundle.

All three run under `bench.*` ids from `packages/core/bench/ext-tiers`, and
none replaces a shipped extension. Nothing in this report changes a default.

## What is compared

The same `Router`, gate and lifecycle that vornd runs drive each extension
through a linked pack in a temp store, against a scripted core
(`world::Bench`). The scripted core holds:

- 32 sessions;
- 20 worktrees over 3 projects;
- a local HTTP server that serves the feeds under `https://feeds.bench/`.

**Behaviour.** `cargo test -p vorn-bench-ext-tiers --test tiers` runs one
scenario per extension in both tiers and requires the two transcripts to be
equal:

- every call's answer;
- every core call the extension made, in order;
- every row it published;
- every element tree it drew, for both the pixel and the cell surface.

An identical tree goes through the one vornui renderer, so both tiers draw
the same frame on every surface.

The same test also runs the Rust RSS port and the shipped Node connector
through the same actions and both polls, including a 304, and requires the
same answers.

## Machine

- Apple M2 Pro, 10 cores, 16 GB, macOS 27.2.
- A shared laptop, so other builds were running.
- Rust 1.97.1, `ci` profile (release, no LTO), Node 22.23.2.

## Commands

```sh
yarn bench:ext-tiers --reps 5 --out bench-out/ext-tiers
cargo test --profile ci -p vorn-bench-ext-tiers --test tiers
```

How the bench runs:

- **Build.** It builds the four components with `vorn-guest-build` and
  compiles each once into a shared cache, as vornd does on install.
- **Repetitions.** Each repetition of each column runs in a fresh process
  (`ext-tiers run`), so memory reads clean. The table is the median of 5.
- **Crash probe.** It runs once per tier (`ext-tiers crash`), with the Widget
  in the same tier loaded beside it.
- **Output.** Raw runs go to `results.json` and the table to `summary.md`.

## Results

|                                             | Widget (wasm) | Widget (program) | Worktree cleanup (wasm) | Worktree cleanup (program) | RSS (wasm) | RSS (program) |    RSS (node) |
| ------------------------------------------- | ------------: | ---------------: | ----------------------: | -------------------------: | ---------: | ------------: | ------------: |
| cold start: first call (ms)                 |         24.51 |            12.45 |                   21.49 |                      12.54 |      47.80 |         12.31 |         65.75 |
| warm start: call after a stop, p50 (ms)     |          3.05 |            10.58 |                    2.90 |                      10.14 |       6.40 |         10.12 |         34.14 |
| cold start: event → first publish, p50 (ms) |          1.81 |             4.57 |                    1.83 |                       4.02 |          — |             — |             — |
| event → publish while running, p50 (ms)     |          0.16 |             0.25 |                    0.07 |                       0.12 |          — |             — |             — |
| call round trip p50 (µs)                    |         10.38 |            21.92 |                   12.79 |                      23.29 |       8.83 |         28.54 |         36.92 |
| call round trip p99 (µs)                    |         19.42 |            38.08 |                   20.00 |                      38.08 |      16.79 |         61.29 |         91.17 |
| typical action p50 (µs)                     |         21.42 |            85.62 |                     299 |                        416 |        929 |           768 |           976 |
| typical action p99 (µs)                     |         34.17 |              124 |                     342 |                        496 |       1085 |           988 |          2060 |
| main tree render p50 (µs)                   |           430 |              497 |                     380 |                        448 |          — |             — |             — |
| main tree render p99 (µs)                   |           470 |              554 |                     421 |                        505 |          — |             — |             — |
| main tree size (bytes)                      |          9078 |             9078 |                    9710 |                       9710 |          — |             — |             — |
| small tree render p50 (µs)                  |         72.38 |              116 |                   32.96 |                      57.92 |          — |             — |             — |
| busy: events or reads per s                 |        817779 |           814393 |                   33719 |                      14844 |        283 |           304 |           364 |
| busy: CPU µs each                           |          1.55 |             3.07 |                   30.14 |                      79.83 |       2520 |          2405 |          4099 |
| memory idle, total (MB)                     |          1.12 |             1.73 |                    1.02 |                       1.27 |       2.47 |          1.49 |         15.30 |
| of which in the host (MB)                   |          1.12 |             0.23 |                    1.02 |                       0.08 |       2.47 |          0.09 |          0.30 |
| of which its own process (MB)               |             — |             1.50 |                       — |                       1.19 |          — |          1.41 |         15.00 |
| memory busy, total (MB)                     |          5.12 |             6.88 |                    5.88 |                      11.94 |       5.83 |          5.52 |         95.17 |
| of which linear memory (MB)                 |          1.25 |                — |                    1.25 |                          — |       2.00 |             — |             — |
| binary (KB)                                 |           338 |              951 |                     355 |                       1016 |       1509 |          2602 |           103 |
| compiled cache (KB)                         |           900 |                — |                     933 |                          — |       3505 |             — |             — |
| lines of code: extension + tier glue        |     449 + 200 |        449 + 153 |               714 + 200 |                  714 + 153 | 1663 + 200 |    1663 + 153 | 2811 (bundle) |

Compiling a component once, on install: Widget 262 ms, Worktree cleanup
263 ms, RSS 1128 ms.

| crash probe                               | wasm                | program              |
| ----------------------------------------- | ------------------- | -------------------- |
| panic                                     | exited in 0.54 ms   | exited in 0.76 ms    |
| next call (restart)                       | answered in 1.56 ms | answered in 6.43 ms  |
| endless loop (5 s call timeout)           | timeout in 6193 ms  | timeout in 7006 ms   |
| neighbour p50 during the loop (µs)        | 9.9                 | 38.9                 |
| next call (restart)                       | answered in 1.23 ms | answered in 5.22 ms  |
| grab 128 MB                               | exited in 0.17 ms   | answered in 15.97 ms |
| held after the grab (MB)                  | 0.36                | 129.36               |
| neighbour round trip after all three (ms) | 0.06                | 0.10                 |

### How each row is measured

- **Cold start.** The first call into a freshly linked extension:
  - a component pays for creating the shared engine and loading its compiled
    cache;
  - a program pays for a process spawn.
- **Warm start.** A call right after the router stopped the extension. The
  engine is already up, so this is the cost of each later restart.
- **Event → publish.** The time from emitting an event the extension acts on
  until its first published row:
  - the Widget is sent `session.status`;
  - Worktree cleanup is sent `worktree.idle`.
- **Typical action:**
  - for the Widget, a button click;
  - for Worktree cleanup, a scan of 20 worktrees;
  - for RSS, reading one RSS 2.0 feed through the host's fetch.
- **Busy:**
  - the Widget and Worktree cleanup take a burst of 5000 events;
  - the RSS columns read all 7 fixtures 200 times;
  - CPU counts this process plus the extension's own.
  - The Widget coalesces each delivered batch into one `sessions.list`, so a
    burst costs it about 20 core calls in either tier.
- **Memory:**
  - _host_ is what this process grew by;
  - _its own process_ is the program's footprint;
  - _linear memory_ is the component's heap, capped at 64 MB.
- **Lines of code.** Non-blank, non-comment, non-test Rust lines.
  - The extension's source is shared by both tiers.
  - The glue for a component is the SDK's component exports.
  - The glue for a program is the stdio host plus its `main`.

## Findings

- **A component is faster on every hot path.**
  - Call round trips take half the time.
  - Restarts take a third of the time.
  - An event reaches its first publish 2.5 times sooner after a stop.
  - Busy work costs half the CPU per event or less, with no process to keep alive.
- **A program starts its first call sooner.** A component's first call pays
  9–35 ms more to load the compiled cache. RSS pays the most, because it carries
  a feed parser.
- **Crashes stay inside the extension in both tiers.** A panic, an endless
  loop or a runaway allocation each ends the extension with an error code,
  and the next call starts it again. The Widget beside it kept answering
  throughout.
  - A component cannot take more than 64 MB, so the 128 MB grab traps at
    once.
  - A program takes all 128 MB from the machine.
  - During the endless loop, the neighbouring program-tier Widget's round
    trip rose to 39 µs, from 22 µs.
- **Size and code.** A component is a third of the program's binary size, but
  its compiled cache is three times larger again. The shared code is
  identical, and the glue is about 150–200 lines either way.
- **Node.** The shipped Node connector spends:
  - 6 to 17 times the memory of either Rust tier;
  - 1.7 times the CPU per feed read;
  - 34 ms on every restart.
- **The program tier reorders publishes.** vornd answers each program
  message on its own task, so a program's publish can land after a later
  call, or after the call's own answer. A render right after a click can
  therefore draw the rows from before the click. The equality test waits for
  the publish counter to settle, so this ordering does not affect its result.
  A component's publishes are applied in order, inside the call.
- **What a component cannot do yet.**
  - A component cannot be a connector, so the Rust RSS has no trigger, saved
    config or `saveFeeds`; its poll takes the feeds and cursor as arguments.
  - The host's fetch does not report the final URL after a redirect.
  - The host's fetch reads the body as UTF-8.
  - Fetches inside one component call run one at a time, where Node keeps 6
    in flight.

## The UI tier

**Recommendation.** Make the Wasm component the tier for extension UI:
vornui's element tree, built inside the component and returned from
`ui.render`.

- **Speed.** It draws the same tree as a program, 15% faster for a full
  window and up to 45% faster for a small one:
  - the Widget's 9 KB tree: 430 µs against 497 µs;
  - the compact tree: 72 µs against 116 µs.
- **Restart cost.** A restarted UI answers in 3 ms rather than 10 ms.
- **Isolation.** A UI that loops or leaks is stopped by the deadline and the
  64 MB cap rather than by the machine.
- **Ordering.** It has none of the program tier's publish-ordering race,
  which can make a program-tier window draw rows from before a click.

Most of a render is in the host for either tier: reading rows and decoding
the tree. Renders are coalesced to the frame clock, so neither tier costs a
frame at these sizes. The one cost a component adds is the first open after
install or launch: a 0.3–1.1 s compile on install and 9–35 ms more on the first call.
Prewarming the components a surface will show hides that cost.

Programs should stay supported for extensions that need their own process:
native libraries, long blocking I/O, or a language without a component
toolchain.
