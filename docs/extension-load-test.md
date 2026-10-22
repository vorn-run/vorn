# Extension load test

20 extensions installed beside 32 busy terminals, against the same run with
none. The numbers set the default idle time and service cap
(`Policy::default()` in `vorn-extensions`): **60 s idle, 24 services**.

## Setup

- **Machine.** GCP n2-standard-16 Spot VMs: 16 vCPU Intel Xeon at 2.80 GHz,
  64 GB, Ubuntu 24.04, kernel 7.0. There is no GPU, so vorn-app draws with
  its CPU raster (llvmpipe) at 1600×1000. The test ran twice, on two VMs of
  that type (A and B), each building the branch in the `ci` profile.
- **Terminals.** 32 panes in the `agent` case of the headless many-pane rig
  (12 shown), each streaming through vornd for 20 s per run.
- **Extensions.** The bench writes 20 extensions (`bench.*`) from the
  templates in `packages/core/bench/ext-load/templates` into a temp HOME and
  links them into a vornd started on it:
  - 15 services: 4 Node, 3 Python, 2 Bash, 2 Rust programs and 4 Rust Wasm
    components. All but the Bash ones also hear session events.
  - 5 one-shots, a process per call: 1 Node, 1 Python, 2 Bash, 1 Rust.
  - Vorn's shipped set, which vornd installs on first start, in both runs.
  - A driver calls one extension at a time, round robin, every extension
    once every 2 s. A service's call asks Vorn for its sessions.
- **Frame time and input latency** come from the vorn-app headless rig: frame
  time per drawn frame, and input latency from a key press to its pixel
  (about 225 presses a run).
- **Memory** is resident size: vornd and sessiond, and separately the
  extension programs that vornd started.
- **Service starts, restarts and stops** are counted from vornd's log.
- **Noise.** Runs with and without extensions alternate. A 20-extension
  median is within noise when its distance from the baseline's is no more
  than the wider min–max range of the two groups.

## Results

Each cell is the median over the repetitions, with the range in brackets.
These are the runs after extension programs stopped forking vornd (below);
the runs before it are under "Input latency, before and after".

### 0 against 20 extensions

At the default idle time and cap, 5 runs a side:

| Measure                      |     A: 0 extensions |        A: 20 extensions |     B: 0 extensions |        B: 20 extensions |
| ---------------------------- | ------------------: | ----------------------: | ------------------: | ----------------------: |
| Frame p50, ms                | 11.23 (11.07–11.31) |     11.25 (11.10–11.33) | 12.33 (12.26–12.44) |     12.24 (12.13–12.45) |
| Frame p99, ms                | 13.79 (13.54–14.02) |     13.91 (13.69–14.23) | 15.52 (15.30–16.00) |     15.40 (15.13–15.45) |
| Input p50, ms                | 16.47 (16.39–16.51) |     16.39 (16.32–16.44) | 16.40 (16.34–16.53) |     16.54 (16.38–16.57) |
| Input p99, ms                | 29.37 (28.43–30.19) |     28.69 (28.45–29.27) | 31.33 (29.88–56.88) |     31.67 (30.28–35.28) |
| Extension call p50, ms       |                   – |        1.33 (1.32–1.36) |                   – |        1.47 (1.45–1.48) |
| Extension call p99, ms       |                   – |           149 (143–151) |                   – |           169 (159–174) |
| RSS vornd, MB                |               291.3 |                   291.3 |               291.4 |                   288.2 |
| RSS programs, MB             |                   0 |                   231.6 |                   0 |                   230.4 |
| Running at peak              |          4 (Vorn's) | 19 (15 bench, 4 Vorn's) |          4 (Vorn's) | 19 (15 bench, 4 Vorn's) |
| Restarts, crashes, cap stops |             0, 0, 0 |                 0, 0, 0 |             0, 0, 0 |                 0, 0, 0 |

The idle phase below repeats the pair (3 runs a side), so each VM gives two
comparisons. Δ is the 20-extension median minus the baseline's, against the
noise band:

| Measure       |   A, default | A, idle phase |   B, default | B, idle phase |
| ------------- | -----------: | ------------: | -----------: | ------------: |
| Frame p50, ms | +0.02 / 0.24 |  +0.17 / 1.14 | −0.09 / 0.32 |  −0.03 / 0.27 |
| Frame p99, ms | +0.12 / 0.54 |  +0.47 / 1.58 | −0.12 / 0.70 |  −0.14 / 0.50 |
| Input p50, ms | −0.08 / 0.12 |   0.00 / 0.08 | +0.14 / 0.19 |  −0.05 / 0.28 |
| Input p99, ms | −0.68 / 1.76 |  −0.13 / 4.17 | +0.34 / 27.0 |  −0.30 / 1.30 |

- Every measure is within noise in all four comparisons.
- B's input p99 band is wide because one run without extensions had a
  56.88 ms p99; without that run the band is 5.40 ms.
- A press is late when its echo misses the frame it would have made (over
  25 ms). Late presses are 3.1% (A) and 4.9% (B) with 20 extensions, against
  3.5% and 3.9% without. With about 225 presses a run, the p99 is the third
  slowest press, which is one of these late presses.

### Input latency, before and after

**Before.** In the first runs (same VM type, same bench) input p99 rose in
all four comparisons: +8.37 ms (band 6.68) and +6.61 ms (band 2.59) on VM A,
outside noise, and +2.83 ms (band 7.20) and +5.79 ms (band 6.39) on B. On A
every 20-extension run was slower at p99 than every run without. 12 of the
18 slow presses on B overlapped a one-shot call.

**Cause.** vornd started each extension program by forking itself. A
manifest names its program bare (`node`, `python3`, `bash`), and the child
gets its own `PATH`. In that case Rust's standard library cannot use
`posix_spawn`: it forks vornd and searches the `PATH` in the child. Forking a
290 MB, many-threaded vornd copies its page tables while holding its
memory-map lock, so every vornd thread that faults a page waits. That
includes the threads that carry a key press to the session and its echo
back to the app. A one-shot extension starts a process on every call, so it
forks vornd on every call.

Measured with bpftrace on one VM, one 20 s run each, with the old start
("forking") and the fixed one:

| vornd, per 20 s run                    | 0, forking | 20, forking | 0, fixed | 20, fixed |
| -------------------------------------- | ---------: | ----------: | -------: | --------: |
| Forks (`clone` syscalls)               |         10 |          58 |       10 |        10 |
| Forks over 4 ms                        |          0 |          22 |        1 |         0 |
| Waits on the memory-map lock over 1 ms |        110 |         289 |      125 |       185 |
| Run-queue waits over 2 ms, all threads |        107 |         149 |       97 |       122 |

- Fixed, extension programs start with `clone3` (vfork) in 0.1–0.5 ms. The
  10 forks left are other things vornd starts, the same with no extensions.
- CPU is not the cause. The app and the daemons together use about 230% of
  the 16 vCPUs. Starting extension programs at nice 10 changed nothing: over
  5 interleaved rounds, 4.0% of presses were late, against 3.7–3.8% without
  it.

**Fix.** vornd looks a bare program name up on the child's own `PATH` and
starts the full path (`Child::start_with` in `vorn-extensions`). The standard
library then uses `posix_spawn`, which shares vornd's memory until the child
execs and takes no page-table copy. A program it cannot find starts as
before and fails with the same ENOENT.

**After.** The tables above: every measure within noise on both VMs. On VM A,
the slower host, input p99 went from +8.37 ms (outside noise) to −0.68 ms.
Of the four comparisons, input p99 now moves −0.68 to +0.34 ms.

**Attribution.** The bench names the call in flight while a press waited
(`input_to_pixel_ms.during`). That favours long calls and long presses: a
30 ms press overlaps more calls than a 16 ms one, so slow presses land
during calls more often whatever the cause. The three slow presses "during
Bash service calls" in the first runs were 100–150 ms presses. Presses that
long happen as often with no extensions (one or two a run), and they span
several calls, so the first one named is arbitrary.

### Service cap, 20 extensions

3 runs a cap (the default's row is the 5 runs above):

| Cap          |    A: frame p50, ms |    B: frame p50, ms | Call p50, ms |   Restarts |  Cap stops |
| ------------ | ------------------: | ------------------: | -----------: | ---------: | ---------: |
| 16           | 11.37 (11.13–11.42) | 12.26 (12.24–12.54) |   1.34, 1.53 | 5 (4–6), 5 | 8 (7–9), 8 |
| 20           | 11.40 (11.16–11.53) | 12.27 (12.25–12.33) |   1.32, 1.50 |          0 |          0 |
| 24 (default) | 11.25 (11.10–11.33) | 12.24 (12.13–12.45) |   1.33, 1.47 |          0 |          0 |
| 32           | 11.26 (11.12–11.37) | 12.30 (12.25–12.35) |   1.35, 1.47 |          0 |          0 |

At cap 16, under the 19 that run at peak, the least recently used services
are stopped and started again.

### Idle phase

At the default 60 s idle time, after the busy run the bench keeps drawing
until every service has stopped (3 runs a side):

| Measure                     |     A: 0 extensions |    A: 20 extensions |     B: 0 extensions |    B: 20 extensions |
| --------------------------- | ------------------: | ------------------: | ------------------: | ------------------: |
| Time until all stopped, s   |    49.6 (49.5–49.7) |    64.3 (64.0–64.9) |    49.5 (49.2–49.5) |    64.4 (64.3–64.7) |
| Running after idle          |                   0 |                   0 |                   0 |                   0 |
| Idle stops                  |          4 (Vorn's) |                  19 |          4 (Vorn's) |                  19 |
| RSS programs after idle, MB |                   0 |                   0 |                   0 |                   0 |
| RSS vornd after idle, MB    | 380.8 (377.8–381.8) | 399.4 (382.1–409.9) | 377.1 (370.7–392.3) | 399.3 (388.2–402.7) |

- Every extension program exits, so their 231 MB is returned.
- vornd keeps 19–22 MB more than the baseline.
- Without extensions, the time is how long Vorn's own services, started by
  the panes' events, take to stop.

### Starting again after idle, by language

VM B, medians (cold call before the fix in brackets):

| Language     | Cold call, ms | Warm call, ms | RSS per process, MB |
| ------------ | ------------: | ------------: | ------------------: |
| Node         | 147.7 (140.6) |          1.37 |                46.6 |
| Python       |   26.2 (38.8) |          1.24 |                10.9 |
| Bash         |    2.7 (18.1) |          0.55 |                 3.6 |
| Rust program |     2.4 (2.3) |          1.02 |                 2.7 |
| Rust Wasm    |     3.1 (2.7) |          0.77 |      (inside vornd) |

- Bash and Python start faster now that vornd no longer forks for them.
- RSS per process is from the first runs.

## Against the bar

- **Frame time and input latency within noise of the run with none: met.**
  Frame p50, frame p99, input p50 and input p99 are within noise in all four
  comparisons, on both VMs.
- **Idle memory recorded and within the cap: met.**
  - 60 s after the last call no service runs (0 of the cap of 24), the
    extension programs hold nothing, and vornd holds about 399 MB against
    377–381 without extensions.

## The defaults

- **Cap 24.** 20 busy extensions plus Vorn's own peak at 19 running services.
  A cap of 20 is the smallest tested that never stops one; 24 leaves room for
  five more. A cap under the working set stops and restarts services all
  run.
- **Idle 60 s.** A running Node service holds about 47 MB, and the next call
  after a stop costs about 140–160 ms for Node and under 3 ms for Rust. Every
  program's memory is back about 65 s after the last call.

## Reproduce

On a GCP Spot VM (the VM and its disks are deleted however the run ends):

```sh
PROJECT=<project> packages/core/bench/ext-load/gcp.sh /tmp/ext-load
```

On a Linux or macOS host with the repo checked out and the shipped packs
fetched (`node packages/core/scripts/shipped.mjs`):

```sh
cd packages/core && bench/ext-load/run.sh /tmp/ext-load
```

A single comparison with the many-pane rig:

```sh
cd packages/core
cargo build --profile ci -p vornd -p vorn-sessiond -p vorn-cli --bins
cargo build --profile ci -p vorn-bench-ext-tiers --bin tier-load
cargo build --profile ci -p vorn-app --example many_panes
target/ci/examples/many_panes --panes 32 --cases agent --seconds 20 \
  --extensions 0,20 --reps 5 --ext-period 2 --idle-wait 150
```

- `--ext-cap N` and `--ext-idle-ms MS` override the defaults for one run.
- `--idle-wait 0` skips the idle phase.
- The results JSON has each press's latency under the kind of call it waited
  behind (`input_to_pixel_ms.during`).
