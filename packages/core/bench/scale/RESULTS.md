# Scale bench results

How the session holder (`vorn-sessiond`) and vornd hold up with many
terminals. Each tier is N sessions in all: one probe, a tenth printing a
build log, and the rest idle. Every section below is one run of
`scripts/bench-gcp.sh` on a fresh Spot VM; the raw JSON, logs and host
limits sit beside this file under `results/<date>-<commit>/`.

- **holder alone**: the bench starts `vorn-sessiond` and drives it over its
  socket as vornd does. Idle sessions run `cat`, so the numbers are the
  holder's own cost. Handoff starts a second holder in the same home and
  times `Adopt` until every live session has moved.
- **vornd + holder**: the bench starts vornd, which launches its holder.
  Sessions are started on vornd's app channel and followed over the grid
  endpoint, as the app does. Idle sessions run `bash`.
- **per idle terminal** is the RSS growth from no sessions to all idle ones,
  divided by the idle count; **per busy terminal** is the growth once the
  busy ones run, divided by the busy count. **Machine memory per idle
  terminal** is the drop in `MemAvailable` over the same step, so it
  includes the programs and the kernel's pty and thread costs.
- **probe echo** is a keystroke written to the probe until its echo comes
  back: as holder records (holder alone) or as a grid frame (vornd +
  holder).
- **throughput** is 16 sessions each printing 16 MiB as fast as they can,
  all attached to one grid client, timed until the client's screens show
  every log's last line.
- **comparison method** (`vorn-scale-bench compare`), run on the build
  under test ("after") and on the merge base with `origin/main` ("before"),
  each with a fresh vornd per terminal count. Every terminal runs `dash`
  (one count also `bash`) and first prints 1,000 numbered lines. Memory is
  `RssAnon` summed over vornd, the holder and every process they started,
  once the terminals are quiet ("live") and again after they sat 60 s
  untouched and the sum stopped moving ("idle"). Response time is from
  typing `echo` into one of the terminals until its output is on that
  terminal's grid screen, beside 0, 10 and 100 terminals running `date` in
  a 0.1 s sleep loop, with the whole machine's CPU use over the same span.

Run it with `PROJECT=<gcp-project> scripts/bench-gcp.sh [TIER...]`. Tiers above 100 refuse to run
unless `VORN_BENCH_HOST=1` is set, which only the VM's runner does.

### 100 sessions · 2026-10-08 · `1f3cff49` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone             | vornd + holder                                                  |
| -------------------------------------------------- | ------------------------ | --------------------------------------------------------------- |
| sessions (idle + busy)                             | 89 + 10                  | 89 + 10                                                         |
| spawn rate (each p50 / p99)                        | 308/s (228.4 / 288.9 ms) | 298/s (114.0 / 120.8 ms)                                        |
| holder RSS idle / busy                             | 12.7 MiB / 15.0 MiB      | 12.8 MiB / 15.2 MiB                                             |
| holder per idle / per busy terminal                | 107.8 KiB / 238.4 KiB    | 104.5 KiB / 242.0 KiB                                           |
| holder threads idle / busy                         | 271 / 301                | 271 / 301                                                       |
| holder fds idle / busy                             | 461 / 511                | 461 / 511                                                       |
| vornd RSS idle / busy                              | —                        | 24.9 MiB / 29.7 MiB                                             |
| vornd per idle / per busy terminal                 | —                        | 126.1 KiB / 483.2 KiB                                           |
| vornd threads idle / busy                          | —                        | 28 / 26                                                         |
| vornd fds idle / busy                              | —                        | 106 / 116                                                       |
| machine memory per idle terminal, program included | —                        | 1.2 MiB                                                         |
| probe echo p50 / p99, others idle                  | 0.33 / 0.43 ms           | 0.73 / 0.87 ms                                                  |
| probe echo p50 / p99, 10 % streaming               | 0.25 / 0.37 ms           | 0.57 / 0.75 ms                                                  |
| attach to snapshot p50 / p99                       | —                        | 0.44 / 0.81 ms                                                  |
| throughput to one grid client                      | —                        | 31.3 MB/s over 16 sessions, 89 frames/s (116.3 KiB/s of frames) |
| handoff of every live session                      | 100 sessions in 48 ms    | —                                                               |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

### 1,000 sessions · 2026-10-08 · `1f3cff49` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone              | vornd + holder                                                 |
| -------------------------------------------------- | ------------------------- | -------------------------------------------------------------- |
| sessions (idle + busy)                             | 899 + 100                 | 899 + 100                                                      |
| spawn rate (each p50 / p99)                        | 52/s (1227.2 / 2277.8 ms) | 51/s (620.9 / 1183.5 ms)                                       |
| holder RSS idle / busy                             | 52.5 MiB / 78.2 MiB       | 54.1 MiB / 79.0 MiB                                            |
| holder per idle / per busy terminal                | 56.1 KiB / 262.4 KiB      | 57.5 KiB / 254.9 KiB                                           |
| holder threads idle / busy                         | 2,701 / 3,001             | 2,701 / 3,001                                                  |
| holder fds idle / busy                             | 4,511 / 5,011             | 4,511 / 5,011                                                  |
| vornd RSS idle / busy                              | —                         | 84.0 MiB / 122.3 MiB                                           |
| vornd per idle / per busy terminal                 | —                         | 80.1 KiB / 392.9 KiB                                           |
| vornd threads idle / busy                          | —                         | 26 / 26                                                        |
| vornd fds idle / busy                              | —                         | 916 / 1,016                                                    |
| machine memory per idle terminal, program included | —                         | 1.2 MiB                                                        |
| probe echo p50 / p99, others idle                  | 0.35 / 0.50 ms            | 0.75 / 0.87 ms                                                 |
| probe echo p50 / p99, 10 % streaming               | 0.18 / 0.29 ms            | 0.48 / 0.63 ms                                                 |
| attach to snapshot p50 / p99                       | —                         | 0.83 / 1.33 ms                                                 |
| throughput to one grid client                      | —                         | 33.5 MB/s over 16 sessions, 67 frames/s (86.9 KiB/s of frames) |
| handoff of every live session                      | 1,000 sessions in 234 ms  | —                                                              |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

### 10,000 sessions · 2026-10-08 · `1f3cff49` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone                | vornd + holder            |
| -------------------------------------------------- | --------------------------- | ------------------------- |
| sessions (idle + busy)                             | 8,999 + 1,000               | 8,999 + 1,000             |
| spawn rate (each p50 / p99)                        | 6/s (11089.8 / 26360.8 ms)  | 6/s (5815.7 / 11357.1 ms) |
| holder RSS idle / busy                             | 362.2 MiB / 4.0 GiB         | 378.7 MiB / —             |
| holder per idle / per busy terminal                | 40.8 KiB / 3.8 MiB          | 42.7 KiB / —              |
| holder threads idle / busy                         | 27,001 / 30,001             | 27,001 / —                |
| holder fds idle / busy                             | 45,011 / 50,411             | 45,011 / —                |
| vornd RSS idle / busy                              | —                           | 684.3 MiB / —             |
| vornd per idle / per busy terminal                 | —                           | 76.3 KiB / —              |
| vornd threads idle / busy                          | —                           | 26 / —                    |
| vornd fds idle / busy                              | —                           | 9,016 / —                 |
| machine memory per idle terminal, program included | —                           | 1.3 MiB                   |
| probe echo p50 / p99, others idle                  | 0.38 / 0.52 ms              | 1.66 / 3.20 ms            |
| probe echo p50 / p99, 10 % streaming               | 0.17 / 0.26 ms              | —                         |
| attach to snapshot p50 / p99                       | —                           | 5.90 / 7.91 ms            |
| throughput to one grid client                      | —                           | —                         |
| handoff of every live session                      | 10,000 sessions in 14618 ms | —                         |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

Limits hit:

- stack at 10,000 sessions: timed out waiting for the echo

Findings from this run:

- **Spawn rate falls with N.** 308/s at 100, 52/s at 1,000, 6/s at 10,000; filling the 10,000 tier took 27 minutes per phase, with sessiond's spawning thread at about half a core and the machine otherwise idle. sessiond starts each program with a `pre_exec` hook, which makes the standard library `fork()` rather than `posix_spawn`, and a fork copies the page tables of a process that holds three threads per session (27,000 thread stacks at 9,000 sessions). The likely fix is a spawn path without `pre_exec`, or one helper process that forks on sessiond's behalf.
- **Echo under load at 10,000.** With 1,000 sessions streaming 16 KiB/s each (16 MB/s together) next to 9,000 idle `bash`, a keystroke on the probe terminal did not come back to the grid client within 30 s, so the busy probe, busy memory and throughput for vornd + holder at 10,000 are missing. At the same load the holder alone echoed in 0.26 ms p99, so the delay is in vornd or in the one holder connection the probe shares with the streams. The busy usage is now read before the probe, so the next run records it either way.
- **Busy holder memory at 10,000 is the delivery window.** 3.8 MiB per busy terminal matches sessiond's 4 MiB per-session window (`WINDOW_BYTES`): one client watching 1,000 streams fell behind and every busy session sat at a full window.
- **Threads: three per session in the holder** (27,001 at 9,000 idle, 30,001 with 1,000 more busy); vornd stays at 26–28 threads at every tier.
- **Handoff grows faster than N:** 48 ms at 100, 234 ms at 1,000, 14.6 s at 10,000.
- Spot VMs in us-central1-a were preempted twice within 20 minutes; this run used us-central1-c. The driver now copies finished tiers back on every poll.

### 1,000 sessions · 2026-10-09 · `9b69effc` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone            | vornd + holder          |
| -------------------------------------------------- | ----------------------- | ----------------------- |
| sessions (idle + busy)                             | 899 + 100               | 899 + 100               |
| spawn rate (each p50 / p99)                        | 3832/s (14.1 / 35.8 ms) | 2454/s (12.0 / 19.8 ms) |
| holder RSS idle / busy                             | 5.2 MiB / 20.4 MiB      | 7.0 MiB / 14.7 MiB      |
| holder per idle / per busy terminal                | 2.3 KiB / 155.3 KiB     | 3.8 KiB / 79.0 KiB      |
| holder threads idle / busy                         | 2 / 2                   | 2 / 2                   |
| holder fds idle / busy                             | 3,614 / 4,014           | 3,614 / 4,014           |
| vornd RSS idle / busy                              | —                       | 87.1 MiB / 110.6 MiB    |
| vornd per idle / per busy terminal                 | —                       | 82.7 KiB / 240.7 KiB    |
| vornd threads idle / busy                          | —                       | 28 / 26                 |
| vornd fds idle / busy                              | —                       | 917 / 1,017             |
| machine memory per idle terminal, program included | —                       | 689.7 KiB               |
| probe echo p50 / p99, others idle                  | 0.22 / 0.34 ms          | 0.46 / 0.66 ms          |
| probe echo p50 / p99, 10 % streaming               | 0.13 / 0.23 ms          | 0.31 / 0.49 ms          |
| attach to snapshot p50 / p99                       | —                       | 0.33 / 0.55 ms          |
| throughput to one grid client                      | —                       | —                       |
| handoff of every live session                      | 1,000 sessions in 83 ms | —                       |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

Limits hit:

- stack at 1,000 sessions: timed out waiting for a snapshot

An early check of this branch: the flood attached to a session vornd had opened before its engine worker had, and was told there was no such session. Fixed before the run below.

### 100 sessions · 2026-10-09 · `af5ca915` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone            | vornd + holder                                                  |
| -------------------------------------------------- | ----------------------- | --------------------------------------------------------------- |
| sessions (idle + busy)                             | 89 + 10                 | 89 + 10                                                         |
| spawn rate (each p50 / p99)                        | 2005/s (37.7 / 37.7 ms) | 1587/s (22.8 / 23.6 ms)                                         |
| holder RSS idle / busy                             | 3.8 MiB / 5.2 MiB       | 4.0 MiB / 4.6 MiB                                               |
| holder per idle / per busy terminal                | 6.7 KiB / 151.6 KiB     | 4.6 KiB / 64.8 KiB                                              |
| holder threads idle / busy                         | 2 / 2                   | 2 / 2                                                           |
| holder fds idle / busy                             | 374 / 414               | 374 / 414                                                       |
| vornd RSS idle / busy                              | —                       | 24.4 MiB / 28.3 MiB                                             |
| vornd per idle / per busy terminal                 | —                       | 113.5 KiB / 393.2 KiB                                           |
| vornd threads idle / busy                          | —                       | 28 / 26                                                         |
| vornd fds idle / busy                              | —                       | 107 / 117                                                       |
| machine memory per idle terminal, program included | —                       | 393.9 KiB                                                       |
| probe echo p50 / p99, others idle                  | 0.30 / 0.43 ms          | 0.75 / 0.93 ms                                                  |
| probe echo p50 / p99, 10 % streaming               | 0.32 / 0.44 ms          | 0.67 / 0.88 ms                                                  |
| attach to snapshot p50 / p99                       | —                       | 0.41 / 0.65 ms                                                  |
| throughput to one grid client                      | —                       | 93.8 MB/s over 16 sessions, 1671 frames/s (2.1 MiB/s of frames) |
| handoff of every live session                      | 100 sessions in 26 ms   | —                                                               |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

### 1,000 sessions · 2026-10-09 · `af5ca915` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone             | vornd + holder                                                  |
| -------------------------------------------------- | ------------------------ | --------------------------------------------------------------- |
| sessions (idle + busy)                             | 899 + 100                | 899 + 100                                                       |
| spawn rate (each p50 / p99)                        | 2740/s (23.3 / 33.9 ms)  | 1912/s (16.3 / 25.4 ms)                                         |
| holder RSS idle / busy                             | 5.1 MiB / 20.4 MiB       | 7.0 MiB / 14.7 MiB                                              |
| holder per idle / per busy terminal                | 2.3 KiB / 157.1 KiB      | 3.8 KiB / 79.0 KiB                                              |
| holder threads idle / busy                         | 2 / 2                    | 2 / 2                                                           |
| holder fds idle / busy                             | 3,614 / 4,014            | 3,614 / 4,014                                                   |
| vornd RSS idle / busy                              | —                        | 85.2 MiB / 111.0 MiB                                            |
| vornd per idle / per busy terminal                 | —                        | 80.0 KiB / 264.4 KiB                                            |
| vornd threads idle / busy                          | —                        | 28 / 26                                                         |
| vornd fds idle / busy                              | —                        | 917 / 1,017                                                     |
| machine memory per idle terminal, program included | —                        | 733.3 KiB                                                       |
| probe echo p50 / p99, others idle                  | 0.34 / 0.48 ms           | 0.73 / 0.94 ms                                                  |
| probe echo p50 / p99, 10 % streaming               | 0.17 / 0.38 ms           | 0.44 / 0.68 ms                                                  |
| attach to snapshot p50 / p99                       | —                        | 0.45 / 0.69 ms                                                  |
| throughput to one grid client                      | —                        | 90.0 MB/s over 16 sessions, 1713 frames/s (2.2 MiB/s of frames) |
| handoff of every live session                      | 1,000 sessions in 103 ms | —                                                               |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

### 10,000 sessions · 2026-10-09 · `af5ca915` · n2-standard-16 (16 vCPU, 62.8 GiB, Linux 7.0.0-1011-gcp)

|                                                    | holder alone               | vornd + holder                                                  |
| -------------------------------------------------- | -------------------------- | --------------------------------------------------------------- |
| sessions (idle + busy)                             | 8,999 + 1,000              | 8,999 + 1,000                                                   |
| spawn rate (each p50 / p99)                        | 1216/s (55.6 / 89.7 ms)    | 995/s (30.9 / 55.8 ms)                                          |
| holder RSS idle / busy                             | 18.0 MiB / 350.8 MiB       | 35.5 MiB / 295.1 MiB                                            |
| holder per idle / per busy terminal                | 1.7 KiB / 340.8 KiB        | 3.6 KiB / 265.9 KiB                                             |
| holder threads idle / busy                         | 2 / 2                      | 2 / 2                                                           |
| holder fds idle / busy                             | 36,014 / 40,014            | 36,014 / 40,014                                                 |
| vornd RSS idle / busy                              | —                          | 686.5 MiB / 1.1 GiB                                             |
| vornd per idle / per busy terminal                 | —                          | 76.5 KiB / 424.1 KiB                                            |
| vornd threads idle / busy                          | —                          | 26 / 26                                                         |
| vornd fds idle / busy                              | —                          | 9,017 / 10,017                                                  |
| machine memory per idle terminal, program included | —                          | 999.3 KiB                                                       |
| probe echo p50 / p99, others idle                  | 0.38 / 0.48 ms             | 0.82 / 1.84 ms                                                  |
| probe echo p50 / p99, 10 % streaming               | 0.12 / 0.24 ms             | 0.41 / 1.94 ms                                                  |
| attach to snapshot p50 / p99                       | —                          | 0.40 / 0.99 ms                                                  |
| throughput to one grid client                      | —                          | 67.2 MB/s over 16 sessions, 1768 frames/s (2.3 MiB/s of frames) |
| handoff of every live session                      | 10,000 sessions in 1666 ms | —                                                               |

Idle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at 16.0 KiB/s each. Throughput: 16 sessions printing 16.0 MiB each as fast as they can.

Findings from this run:

- **Spawn no longer falls with N:** 995/s at 10,000 through vornd (6/s before). sessiond starts programs with `posix_spawn`, so nothing forks a large process.
- **Holder threads are flat:** 2 at every tier (30,001 before). Every terminal is read and written by one poll thread; the holder's fds are four per terminal on Linux (master, a read and a write duplicate, a pidfd).
- **Echo under load:** 1.94 ms p99 through vornd to a grid client with 1,000 of 10,000 sessions streaming (no echo within 30 s before). Replies and input acks overtake queued records on the holder connection, records go in 256 KiB frames, and vornd's resize timers and session lookups no longer walk every session per message.
- **A slow client holds at most 64 MiB of a connection's unacked records**, not 4 MiB per session: 341 KiB per busy terminal with the holder alone at 10,000 (3.8 MiB before), 266 KiB through vornd.
- **Handoff is about linear:** 26 ms at 100, 103 ms at 1,000, 1.67 s at 10,000 (14.6 s before).
