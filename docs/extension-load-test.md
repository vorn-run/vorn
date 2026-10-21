# Extension load test

20 extensions installed beside 32 busy terminals, against the same run with
none. The numbers set the default idle time and service cap
(`Policy::default()` in `vorn-extensions`): **60 s idle, 24 services**.

## Setup

- **Machine.** GCP n2-standard-16: 16 vCPU Intel Xeon at 2.80 GHz, 64 GB,
  Ubuntu 24.04, kernel 7.0. There is no GPU, so vorn-app draws with its CPU
  raster (llvmpipe) at 1600×1000.
- **Terminals.** 32 panes in the `agent` case of the headless many-pane rig
  (12 shown), each running steady output through vornd for 20 s per run.
- **Extensions.** The bench generates 20 extensions (`bench.*`) into a temp
  dir and links them into a vornd started with a temp HOME and data dir:
  - 16 services: 5 Node, 4 Python, 2 Bash and 5 Rust (Wasm components).
  - 4 one-shot commands.
  - A driver calls one extension at a time, round robin, every extension once
    every 2 s.
- **Frame time and input latency** come from the vorn-app headless rig: frame
  time per drawn frame, and input latency from a key press to its pixel.
- **Memory** is resident size: vornd and sessiond, and separately the
  extension programs that vornd started.
- **Service starts, restarts and stops** are counted from vornd's log.

## Results

Each cell is the median over the repetitions, with the range in brackets.

### 0 against 20 extensions

| Measure                | 0 extensions (5 runs) | 20 extensions, cap 24 (3 runs) |
| ---------------------- | --------------------: | -----------------------------: |
| Frame p50, ms          |      8.27 (8.19–8.46) |               8.41 (8.40–8.53) |
| Frame p99, ms          |   10.28 (10.03–10.49) |            10.65 (10.18–10.66) |
| Input p50, ms          |   16.06 (15.90–16.15) |            15.93 (15.73–16.02) |
| Input p99, ms          |   25.89 (25.34–26.18) |            52.17 (25.48–99.25) |
| Extension call p50, ms |                     – |               1.04 (1.02–1.07) |
| RSS vornd, MB          |                 275.5 |                          281.0 |
| RSS programs, MB       |                     0 |                          284.3 |
| Running at peak        |                     0 |      21 (16 bench, 5 built-in) |
| Restarts, cap stops    |                     – |                           0, 0 |

- Frame p50 rises 0.14 ms, inside the baseline's 0.27 ms spread.
- Frame p99 and input p50 are within noise.
- Input p99 counts as within noise only because the 20-extension spread is so
  wide:
  - The 20-extension runs range from 25 to 99 ms, against a steady 25–26 ms
    with none.
  - One run in three is clean. In the others, a few presses likely land while
    extension programs start (Node's cold start takes over 100 ms of CPU) and
    wait a frame or more.
  - The p50 does not move.
- In the idle phase's paired run, frame p50 is 8.45 against 8.29 ms: Δ +0.16
  ms, just over that run's 0.14 ms spread. Read it as about 2% of a frame spent
  on extension work.

### Service cap, 20 extensions

| Cap     |    Frame p50, ms | Call p50, ms | Restarts | Cap stops |
| ------- | ---------------: | -----------: | -------: | --------: |
| 8 (old) | 8.79 (8.71–8.83) |         26.5 |      146 |       159 |
| 12      | 8.71 (8.66–8.78) |         26.5 |      142 |       151 |
| 16      | 8.49 (8.43–8.50) |         1.06 |        6 |        11 |
| 20      | 8.49 (8.40–8.50) |         1.06 |        0 |         1 |
| 24      | 8.41 (8.40–8.53) |         1.04 |        0 |         0 |

Below the working set, the least recently used service is stopped and
restarted on every call:

- Calls cost a cold start: 26 ms instead of 1 ms.
- Frame p50 rises 0.5 ms, outside noise.

### Idle phase

The idle time is set to 60 s. After the busy run, the bench keeps drawing
until every service has stopped (3 runs per side).

| Measure                     |        0 extensions |             20 extensions |
| --------------------------- | ------------------: | ------------------------: |
| Time until all stopped, s   |                   – |          65.4 (65.3–67.3) |
| Idle stops                  |                   – | 21 (16 bench, 5 built-in) |
| RSS programs after idle, MB |                   0 |                         0 |
| RSS vornd after idle, MB    | 371.6 (367.6–380.2) |       387.0 (378.5–389.6) |

Every extension program exits, so their 284 MB is returned. vornd keeps
about 15 MB more than the baseline: the compiled Wasm components and the
allocator's retained pages.

### Starting again after idle, by language (median)

| Language    | Cold call, ms | Warm call, ms | RSS per process, MB |
| ----------- | ------------: | ------------: | ------------------: |
| Node        |         126.4 |          1.05 |                46.7 |
| Python      |          34.0 |          0.90 |                10.9 |
| Bash        |          11.4 |          0.48 |                 3.6 |
| Rust (Wasm) |           3.0 |          0.25 |      (inside vornd) |

## The defaults

- **Cap 24.** 20 busy extensions plus the built-ins peak at 21 running
  services. A cap of 24 is the smallest tested that never evicts one, with
  room for a few more. A cap under the working set restarts services
  continuously and moves frame time out of noise.
- **Idle 60 s.** A running Node service holds about 47 MB, and the next call
  after a stop costs at most about 130 ms (Node) and 3 ms for Wasm. A minute
  frees that memory without stopping anything that is in active use.

## Reproduce

On a GCP Spot VM (the VM and its disks are deleted however the run ends):

```sh
PROJECT=<project> packages/core/bench/ext-load/gcp.sh /tmp/ext-load
```

On a Linux or macOS host with the repo checked out:

```sh
cd packages/core && bench/ext-load/run.sh /tmp/ext-load
```

A single comparison with the many-pane rig:

```sh
cd packages/core
cargo build --profile ci -p vornd -p vorn-sessiond -p vorn-cli --bins
cargo build --profile ci -p vorn-app --example many_panes
target/ci/examples/many_panes --panes 32 --cases agent --seconds 20 \
  --extensions 0,20 --reps 5 --ext-period 2 --idle-wait 120
```

- `--ext-cap N` and `--ext-idle-ms MS` override the defaults for one run.
- `--idle-wait 0` skips the idle phase.
