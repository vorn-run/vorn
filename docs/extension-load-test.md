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

### 0 against 20 extensions

At the default idle time and cap, 5 runs a side:

| Measure                      |     A: 0 extensions |        A: 20 extensions |     B: 0 extensions |        B: 20 extensions |
| ---------------------------- | ------------------: | ----------------------: | ------------------: | ----------------------: |
| Frame p50, ms                | 11.19 (10.91–11.29) |     11.25 (11.16–11.42) |    9.56 (9.42–9.79) |        9.73 (9.57–9.84) |
| Frame p99, ms                | 13.71 (13.61–14.05) |     14.18 (13.92–14.78) | 11.76 (11.58–11.94) |     12.20 (11.77–12.62) |
| Input p50, ms                | 16.37 (16.28–16.42) |     16.34 (16.19–16.44) | 16.21 (15.96–16.26) |     16.16 (16.12–16.18) |
| Input p99, ms                | 29.40 (28.48–30.19) |     37.77 (32.92–39.60) | 27.25 (26.28–27.39) |     30.08 (25.19–32.39) |
| Extension call p50, ms       |                   – |        1.47 (1.43–1.49) |                   – |        1.18 (1.14–1.26) |
| Extension call p99, ms       |                   – |           159 (154–172) |                   – |           139 (134–148) |
| RSS vornd, MB                |               292.2 |                   288.0 |               293.4 |                   289.6 |
| RSS programs, MB             |                   0 |                   231.9 |                   0 |                   231.7 |
| Running at peak              |          4 (Vorn's) | 19 (15 bench, 4 Vorn's) |          4 (Vorn's) | 19 (15 bench, 4 Vorn's) |
| Restarts, crashes, cap stops |             0, 0, 0 |                 0, 0, 0 |             0, 0, 0 |                 0, 0, 0 |

The idle phase below repeats the pair (3 runs a side), so each VM gives two
comparisons. Δ is the 20-extension median minus the baseline's, against the
noise band:

| Measure       |       A, default |    A, idle phase |   B, default |    B, idle phase |
| ------------- | ---------------: | ---------------: | -----------: | ---------------: |
| Frame p50, ms |     +0.06 / 0.38 |     +0.10 / 0.21 | +0.17 / 0.37 |     +0.09 / 0.29 |
| Frame p99, ms |     +0.47 / 0.86 |     +0.10 / 0.75 | +0.44 / 0.85 | **+0.58 / 0.50** |
| Input p50, ms |     −0.03 / 0.25 |     −0.06 / 0.23 | −0.05 / 0.30 |     −0.14 / 0.18 |
| Input p99, ms | **+8.37 / 6.68** | **+6.61 / 2.59** | +2.83 / 7.20 |     +5.79 / 6.39 |

- Frame p50 and input p50 are within noise in all four comparisons.
- Frame p99 is within noise in three; in the fourth it is 0.08 ms past the
  band.
- Input p99 rises in all four, by 2.8 to 8.4 ms, and is outside noise in
  both comparisons on VM A. On A every 20-extension run is slower at p99
  than every run without extensions.
- VM A was the slower host (frame p50 11.2 ms without extensions, against
  9.6 ms on B).

### Where the slow presses wait

Each press is put down to the extension call in flight while it waited (the
first, when it spans two). On VM B, 18 of the 1130 presses in the
20-extension runs were slower than the pooled p99 of the runs without
extensions (27.4 ms):

| In flight       | Share of presses |                  Slow presses |
| --------------- | ---------------: | ----------------------------: |
| No call         |            75.5% |                             3 |
| A one-shot call |            12.3% | 12 (6 Node, 4 Bash, 2 Python) |
| A service call  |            12.2% |                  3 (all Bash) |

- The tail follows the one-shots. A one-shot starts a process per call. A
  Node one-shot call takes 140–160 ms (the calls' p99 above, against
  1.2–1.5 ms p50), and presses waiting while one-shots run make two-thirds of
  the slow ones.
- Calls to Wasm, Rust, Python and Node services account for none.
- The three slow presses during Bash service calls are not explained by
  these runs.

### Service cap, 20 extensions

VM B, 3 runs a cap (the default's row is the 5 runs above):

| Cap          |    Frame p50, ms | Call p50, ms | Restarts | Cap stops |
| ------------ | ---------------: | -----------: | -------: | --------: |
| 16           | 9.95 (9.82–9.99) |         1.26 |  6 (5–6) |   9 (8–9) |
| 20           | 9.90 (9.79–9.99) |         1.24 |        0 |         0 |
| 24 (default) | 9.73 (9.57–9.84) |         1.18 |        0 |         0 |
| 32           | 9.80 (9.71–9.85) |         1.17 |        0 |         0 |

At cap 16, under the 19 that run at peak, the least recently used services
are stopped and started again. On VM A the same cap gave 5 restarts and 8
cap stops a run.

### Idle phase

At the default 60 s idle time, after the busy run the bench keeps drawing
until every service has stopped (VM B, 3 runs a side):

| Measure                     |        0 extensions |           20 extensions |
| --------------------------- | ------------------: | ----------------------: |
| Time until all stopped, s   |    49.6 (49.4–49.8) |        64.8 (64.7–64.9) |
| Running after idle          |                   0 |                       0 |
| Idle stops                  |          4 (Vorn's) | 19 (15 bench, 4 Vorn's) |
| RSS programs after idle, MB |                   0 |                       0 |
| RSS vornd after idle, MB    | 375.2 (362.9–383.2) |     399.7 (399.0–414.4) |

- Every extension program exits, so their 232 MB is returned.
- vornd keeps 24.5 MB more than the baseline. On VM A the figures were 394.7
  against 383.2 MB (11.5 MB more), with all stopped at 64.6 s.
- Without extensions, the time is how long Vorn's own services, started by
  the panes' events, take to stop.

### Starting again after idle, by language

VM B, medians:

| Language     | Cold call, ms | Warm call, ms | RSS per process, MB |
| ------------ | ------------: | ------------: | ------------------: |
| Node         |         140.6 |          1.25 |                46.6 |
| Python       |          38.8 |          1.43 |                10.9 |
| Bash         |          18.1 |          0.54 |                 3.6 |
| Rust program |           2.3 |          0.84 |                 2.7 |
| Rust Wasm    |           2.7 |          0.54 |      (inside vornd) |

## Against the bar

- **Frame time and input latency within noise of the run with none: not
  met at the tail.**
  - Frame p50, frame p99 and input p50 hold, apart from one frame p99
    comparison 0.08 ms past its band.
  - Input p99 rises 2.8–8.4 ms and leaves the noise band on VM A. The slow
    presses wait behind one-shot calls, Node's most (above).
- **Idle memory recorded and within the cap: met.**
  - 60 s after the last call no service runs (0 of the cap of 24), the
    extension programs hold nothing, and vornd holds 399.7 MB against 375.2
    without extensions.

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
