# Tasks as a built-in, a program and a Wasm component

Machine: {"cpu":"Apple M2 Pro","cores":10,"os":"macos"}. Cells: median of the runs (lowest–highest). One board in a real store per case; the program and the component reach it over the bridge.

**100 tasks**

| Metric                                           |          builtin |          program |             wasm |
| ------------------------------------------------ | ---------------: | ---------------: | ---------------: |
| list_tasks (MCP, every page of task.list) ms p50 | 0.76 (0.73–1.25) | 2.28 (1.67–2.49) | 1.32 (1.25–2.35) |
| list_tasks (MCP, every page of task.list) ms p99 | 1.81 (1.57–3.77) | 5.17 (3.75–7.61) | 2.49 (2.18–6.16) |
| create answer ms p50                             | 0.42 (0.37–0.77) | 1.28 (1.18–1.57) | 0.75 (0.66–1.08) |
| create answer ms p99                             | 2.61 (1.15–4.36) | 6.10 (3.68–11.7) | 2.58 (1.75–6.82) |
| update answer ms p50                             | 0.20 (0.18–0.55) | 1.06 (0.99–1.36) | 0.64 (0.44–1.13) |
| update answer ms p99                             | 1.59 (0.80–3.21) | 5.16 (3.30–7.33) | 3.65 (1.39–7.70) |
| move answer ms p50                               | 0.51 (0.43–0.65) | 1.56 (1.49–2.42) | 0.97 (0.86–2.87) |
| move answer ms p99                               | 2.60 (1.76–3.58) | 5.34 (4.40–13.0) | 2.90 (2.11–11.4) |
| move to badge row published ms p50               | 0.51 (0.43–0.65) | 1.56 (1.49–2.42) | 0.97 (0.86–2.87) |
| move to badge row published ms p99               | 2.60 (1.76–3.58) | 5.34 (4.41–13.0) | 2.90 (2.11–11.4) |
| change broadcast bytes p50                       |    800 (800–800) |    800 (800–800) |    800 (800–800) |
| bridge calls per create                          |                — | 6.00 (6.00–6.00) | 6.00 (6.00–6.00) |
| bridge calls per move                            |                — | 7.00 (7.00–7.00) | 7.00 (7.00–7.00) |
| restart ms p50                                   | 0.11 (0.05–0.15) | 13.4 (12.1–19.9) | 2.60 (2.37–3.56) |
| memory MB at end (tier's own)                    | 1.95 (1.78–1.97) | 6.77 (6.59–6.83) | 3.47 (3.28–3.53) |
| Wasm linear memory MB                            |                — |                — | 1.44 (1.44–1.44) |

**1000 tasks**

| Metric                                           |          builtin |          program |             wasm |
| ------------------------------------------------ | ---------------: | ---------------: | ---------------: |
| list_tasks (MCP, every page of task.list) ms p50 | 7.55 (7.19–8.38) | 14.7 (13.7–15.2) | 12.9 (12.5–13.8) |
| list_tasks (MCP, every page of task.list) ms p99 | 9.63 (8.84–10.7) | 19.7 (17.6–25.6) | 17.8 (14.0–18.8) |
| create answer ms p50                             | 0.46 (0.32–0.50) | 1.14 (1.09–1.39) | 0.84 (0.83–1.13) |
| create answer ms p99                             | 1.89 (1.51–6.46) | 3.44 (2.35–4.32) | 2.94 (1.93–3.84) |
| update answer ms p50                             | 0.31 (0.21–0.35) | 0.86 (0.82–0.93) | 0.60 (0.58–0.65) |
| update answer ms p99                             | 2.34 (1.83–6.07) | 3.00 (2.28–4.11) | 2.87 (1.65–5.45) |
| move answer ms p50                               | 0.68 (0.55–0.71) | 1.68 (1.56–1.73) | 1.22 (1.12–1.29) |
| move answer ms p99                               | 3.85 (3.62–5.31) | 3.66 (3.23–6.93) | 3.03 (2.37–4.28) |
| move to badge row published ms p50               | 0.68 (0.55–0.71) | 1.68 (1.56–1.73) | 1.22 (1.12–1.29) |
| move to badge row published ms p99               | 3.85 (3.62–5.31) | 3.67 (3.23–6.93) | 3.03 (2.37–4.28) |
| change broadcast bytes p50                       |    800 (800–800) |    800 (800–800) |    800 (800–800) |
| bridge calls per create                          |                — | 6.00 (6.00–6.00) | 6.00 (6.00–6.00) |
| bridge calls per move                            |                — | 7.00 (7.00–7.00) | 7.00 (7.00–7.00) |
| restart ms p50                                   | 0.19 (0.13–0.24) | 15.3 (11.9–20.4) | 2.35 (2.05–2.68) |
| memory MB at end (tier's own)                    | 14.3 (13.6–14.7) | 33.6 (33.2–34.1) | 19.0 (19.0–20.3) |
| Wasm linear memory MB                            |                — |                — | 5.50 (5.50–5.50) |

**10000 tasks**

| Metric                                           |          builtin |          program |             wasm |
| ------------------------------------------------ | ---------------: | ---------------: | ---------------: |
| list_tasks (MCP, every page of task.list) ms p50 |    371 (363–420) |    437 (420–541) |    416 (404–503) |
| list_tasks (MCP, every page of task.list) ms p99 |    455 (403–565) |    494 (475–688) |   575 (458–1442) |
| create answer ms p50                             | 3.66 (3.38–4.54) | 5.12 (4.31–7.22) | 4.15 (4.14–5.65) |
| create answer ms p99                             | 8.06 (6.83–10.3) | 10.7 (7.74–14.0) | 8.55 (6.94–26.1) |
| update answer ms p50                             | 0.31 (0.24–0.39) | 0.94 (0.77–1.49) | 0.58 (0.49–0.64) |
| update answer ms p99                             | 1.57 (0.62–4.21) | 4.87 (4.36–6.77) | 1.62 (0.78–2.84) |
| move answer ms p50                               | 1.77 (1.54–2.32) | 4.59 (3.84–5.45) | 2.84 (2.59–3.19) |
| move answer ms p99                               | 5.21 (3.16–6.88) | 7.99 (5.82–16.5) | 4.68 (4.46–13.6) |
| move to badge row published ms p50               | 1.77 (1.54–2.32) | 4.59 (3.84–5.45) | 2.84 (2.59–3.19) |
| move to badge row published ms p99               | 5.21 (3.16–6.88) | 7.99 (5.82–16.5) | 4.68 (4.46–13.6) |
| change broadcast bytes p50                       |    800 (800–800) |    800 (800–800) |    800 (800–800) |
| bridge calls per create                          |                — | 6.00 (6.00–6.00) | 6.00 (6.00–6.00) |
| bridge calls per move                            |                — | 7.00 (7.00–7.00) | 7.00 (7.00–7.00) |
| restart ms p50                                   | 1.15 (1.02–3.11) | 22.3 (16.8–33.5) | 4.03 (3.36–4.29) |
| memory MB at end (tier's own)                    |    186 (185–208) |    534 (533–576) |    220 (220–244) |
| Wasm linear memory MB                            |                — |                — | 42.1 (42.1–42.1) |

**Same board under each tier** (last run of each)

| Tier    | triggers fired                              | badge rows | image upload                                                                     |
| ------- | ------------------------------------------- | ---------: | -------------------------------------------------------------------------------- |
| builtin | {"task.created":60,"task.statusChanged":60} |          4 | stored                                                                           |
| program | {"task.created":60,"task.statusChanged":60} |          4 | refused: no images: the extension API has no blob store, and a component no disk |
| wasm    | {"task.created":60,"task.statusChanged":60} |          4 | refused: no images: the extension API has no blob store, and a component no disk |
