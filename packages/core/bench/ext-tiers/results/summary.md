Machine: Apple M2 Pro, 10 cores, macos; medians of 5 runs.

### Worktree cleanup

|                                            | built-in | program |   Wasm |
| ------------------------------------------ | -------: | ------: | -----: |
| first start (ms)                           |     0.11 |   13.47 |  19.42 |
| restart p50 (ms)                           |     0.00 |    8.28 |   2.35 |
| cold start: event → first publish p50 (ms) |     0.06 |    4.11 |   1.54 |
| call round trip p50 (µs)                   |      0.5 |    26.1 |   16.2 |
| call round trip p99 (µs)                   |      1.2 |    47.6 |   23.4 |
| scan p50 (µs)                              |    116.0 |   264.7 |  214.0 |
| scan p99 (µs)                              |    361.7 |   348.9 |  250.5 |
| 5k worktree.idle: events/s                 |   70,419 |  17,172 | 27,133 |
| 5k worktree.idle: CPU µs/event             |     15.6 |    67.4 |   37.6 |
| memory idle (MB)                           |     0.03 |    1.30 |   0.52 |
| memory busy (MB)                           |     5.66 |   10.22 |   4.70 |
| of which linear memory, busy (MB)          |        — |       — |   1.25 |

### Widget

|                                            |  built-in | program |      Wasm |
| ------------------------------------------ | --------: | ------: | --------: |
| first start (ms)                           |      0.43 |   15.03 |     25.17 |
| restart p50 (ms)                           |      0.14 |    8.81 |      2.44 |
| cold start: event → first publish p50 (ms) |      0.14 |    4.50 |      1.49 |
| call round trip p50 (µs)                   |       0.5 |    26.2 |      15.7 |
| call round trip p99 (µs)                   |       1.3 |    47.6 |      23.7 |
| event → row p50 (µs)                       |     214.7 |   125.6 |     117.8 |
| event → row p99 (µs)                       |     344.5 |   214.4 |     202.2 |
| 20k session.status: events/s               | 1,131,326 | 553,111 | 1,044,139 |
| 20k session.status: CPU µs/event           |       1.8 |     4.6 |       1.6 |
| 20k session.status: guest calls            |        68 |     177 |        80 |
| 5k agent.permission: events/s              |    30,781 |  26,554 |    18,101 |
| 5k agent.permission: CPU µs/event          |      35.9 |    66.3 |      57.8 |
| memory idle (MB)                           |      0.25 |    1.75 |      0.66 |
| memory busy (MB)                           |     52.91 |   54.27 |     43.13 |
| of which linear memory, busy (MB)          |         — |       — |     12.00 |

### Same rows

| extension | steps | program: same rows / same order | Wasm: same rows / same order |
| --------- | ----: | ------------------------------: | ---------------------------: |
| worktrees |    15 |                       5/5 · 5/5 |                    5/5 · 5/5 |
| widget    |    11 |                       5/5 · 5/5 |                    5/5 · 5/5 |

### Crashes

|                                                        | program                 | Wasm                    |
| ------------------------------------------------------ | ----------------------- | ----------------------- |
| endless loop                                           | `timeout` after 4005 ms | `timeout` after 2385 ms |
| grab 256 MB                                            | allowed                 | refused (exited)        |
| restart after a crash p50 (ms)                         | 3.22                    | 1.13                    |
| crashes (the grab included) before the budget holds it | 5                       | 5                       |
| host still answers                                     | yes                     | yes                     |

### Sizes

| artifact                                     |                  size |
| -------------------------------------------- | --------------------: |
| program binary (both extensions)             |                490 KB |
| component .wasm (both extensions)            |                235 KB |
| component .cwasm (precompiled)               |                616 KB |
| host without the Wasm tier                   |              3,385 KB |
| host with its runtime (loads .cwasm)         |  6,185 KB (+2,801 KB) |
| host with runtime and compiler (loads .wasm) | 12,601 KB (+9,216 KB) |

### Lines of code

| part              | lines |
| ----------------- | ----: |
| builtin worktrees |   270 |
| builtin widget    |   300 |
| ported worktrees  |   235 |
| ported widget     |   272 |
| shared logic      |   109 |
| program glue      |   136 |
| component glue    |    69 |
| host wasm         |   604 |
