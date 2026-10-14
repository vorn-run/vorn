# Workflow engine under load

Machine: {"os":"macos","arch":"aarch64","cpu":"Apple M2 Pro","cores":10,"memory_gb":17.0}. Cells: median of the repeated runs (lowest–highest). Failed rows left out: 0. Load average before each case: 7.63 (median); builds by other users may overlap, so the labels take turns case by case.

**1 concurrent runs × 10 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 0.81 (0.50–2.68) | 0.19 (0.11–2.18) | ×0.24 |
| Trigger → first step p99 (ms) | 6.56 (0.99–8.73) | 0.54 (0.44–3.06) | ×0.08 |
| First step seen by a viewer p99 (ms) | 9.66 (2.75–12.8) | 1.05 (0.83–3.74) | ×0.11 |
| Per step p50 (µs) | 100 (100–500) | 100 (100–400) | ×1.00 |
| vornd CPU per step (µs) | 485 (456–1761) | 325 (320–984) | ×0.67 |
| Steps/s | 3308 (809–3352) | 4877 (1374–5066) | ×1.47 |
| Peak memory per run (KB) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |
| Run broadcasts | 220 (220–220) | 220 (220–220) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**1 concurrent runs × 100 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 3.66 (3.60–6.95) | 1.53 (1.19–5.99) | ×0.42 |
| Trigger → first step p99 (ms) | 9.29 (8.33–10.4) | 5.83 (2.91–12.8) | ×0.63 |
| First step seen by a viewer p99 (ms) | 114 (99.6–118) | 6.50 (3.48–14.7) | ×0.06 |
| Per step p50 (µs) | 940 (900–960) | 540 (530–740) | ×0.57 |
| vornd CPU per step (µs) | 1908 (1864–1945) | 1062 (1034–1503) | ×0.56 |
| Steps/s | 884 (867–937) | 1714 (1161–1771) | ×1.94 |
| Peak memory per run (KB) | 32.0 (0.00–96.0) | 32.0 (16.0–128) | ×1.00 |
| Run broadcasts | 2020 (2020–2020) | 2020 (2020–2020) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**10 concurrent runs × 10 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 8.16 (6.99–18.0) | 1.46 (1.25–2.40) | ×0.18 |
| Trigger → first step p99 (ms) | 14.0 (13.2–36.3) | 3.57 (3.44–14.5) | ×0.26 |
| First step seen by a viewer p99 (ms) | 19.7 (17.6–39.6) | 4.93 (4.54–17.6) | ×0.25 |
| Per step p50 (µs) | 400 (400–600) | 600 (600–700) | ×1.50 |
| vornd CPU per step (µs) | 677 (630–932) | 391 (382–534) | ×0.58 |
| Steps/s | 5183 (3518–5707) | 8744 (7252–10165) | ×1.69 |
| Peak memory per run (KB) | 0.00 (0.00–30.4) | 0.00 (0.00–0.00) | – |
| Run broadcasts | 660 (660–660) | 660 (660–660) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**10 concurrent runs × 100 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 57.8 (50.1–62.1) | 5.78 (5.20–6.37) | ×0.10 |
| Trigger → first step p99 (ms) | 115 (89.0–132) | 15.4 (10.2–15.4) | ×0.13 |
| First step seen by a viewer p99 (ms) | 448 (333–508) | 17.7 (11.7–18.4) | ×0.04 |
| Per step p50 (µs) | 4030 (3100–4390) | 2390 (2000–2910) | ×0.59 |
| vornd CPU per step (µs) | 2166 (2019–2187) | 1134 (1035–1306) | ×0.52 |
| Steps/s | 2024 (1831–2487) | 3766 (2852–4588) | ×1.86 |
| Peak memory per run (KB) | 62922 (58222–63022) | 939 (456–5718) | ×0.01 |
| Run broadcasts | 6060 (6060–6060) | 6060 (6060–6060) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**100 concurrent runs × 10 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 353 (331–356) | 8.90 (7.20–10.5) | ×0.03 |
| Trigger → first step p99 (ms) | 640 (629–664) | 20.5 (17.8–21.0) | ×0.03 |
| First step seen by a viewer p99 (ms) | 641 (631–664) | 74.3 (34.4–75.8) | ×0.12 |
| Per step p50 (µs) | 100 (100–200) | 6400 (3600–9700) | ×64.00 |
| vornd CPU per step (µs) | 1301 (1233–1324) | 362 (332–422) | ×0.28 |
| Steps/s | 1539 (1483–1562) | 9392 (5900–10173) | ×6.10 |
| Peak memory per run (KB) | 970 (949–1005) | 434 (320–564) | ×0.45 |
| Run broadcasts | 2200 (2200–2200) | 2200 (2200–2200) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**100 concurrent runs × 100 steps**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → first step p50 (ms) | 3487 (3476–3837) | 45.6 (37.3–55.6) | ×0.01 |
| Trigger → first step p99 (ms) | 6876 (6862–7256) | 81.5 (65.2–92.6) | ×0.01 |
| First step seen by a viewer p99 (ms) | 7058 (6934–7265) | 141 (91.1–149) | ×0.02 |
| Per step p50 (µs) | 1630 (1370–1670) | 28940 (23650–32220) | ×17.75 |
| vornd CPU per step (µs) | 2673 (2432–2743) | 1307 (1135–1432) | ×0.49 |
| Steps/s | 1412 (1343–1421) | 2833 (2476–3192) | ×2.01 |
| Peak memory per run (KB) | 69135 (69050–69314) | 6749 (6142–8138) | ×0.10 |
| Run broadcasts | 20200 (20200–20200) | 20200 (20200–20200) | ×1.00 |
| Runs that did not succeed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Gates, 1 concurrent runs**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → waiting at the gate p99 (ms) | 1.21 (1.20–1.49) | 1.73 (0.85–1.95) | ×1.43 |
| Approve answered p50 (ms) | 0.75 (0.73–0.78) | 0.76 (0.74–0.78) | ×1.02 |
| Approve → next step p50 (ms) | 0.24 (0.21–0.42) | 0.34 (0.01–0.55) | ×1.40 |
| Approve → next step p99 (ms) | 0.92 (0.47–1.15) | 0.78 (0.69–1.16) | ×0.85 |
| Memory per parked run (KB) | 0.00 (0.00–0.00) | 0.00 (0.00–16.0) | – |

**Gates, 10 concurrent runs**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → waiting at the gate p99 (ms) | 5.53 (5.52–7.45) | 2.87 (2.03–2.90) | ×0.52 |
| Approve answered p50 (ms) | 1.26 (1.07–2.46) | 0.83 (0.65–1.43) | ×0.66 |
| Approve → next step p50 (ms) | 0.84 (0.61–2.05) | 0.43 (0.13–0.90) | ×0.51 |
| Approve → next step p99 (ms) | 1.54 (1.20–4.36) | 0.79 (0.61–1.47) | ×0.51 |
| Memory per parked run (KB) | 110 (92.8–142) | 123 (112–131) | ×1.12 |

**Gates, 100 concurrent runs**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Trigger → waiting at the gate p99 (ms) | 261 (246–348) | 18.9 (13.8–36.9) | ×0.07 |
| Approve answered p50 (ms) | 1.17 (1.17–2.54) | 0.98 (0.66–1.75) | ×0.84 |
| Approve → next step p50 (ms) | 0.81 (0.80–2.18) | 0.64 (0.28–1.52) | ×0.80 |
| Approve → next step p99 (ms) | 4.45 (2.08–31.0) | 1.53 (1.07–55.1) | ×0.35 |
| Memory per parked run (KB) | 98.9 (92.0–126) | 76.5 (65.3–79.5) | ×0.77 |

**Schedules: 12 workflows due two seconds apart**

| Metric | before | after | after / before |
| --- | ---: | ---: | ---: |
| Fired | 12.0 (12.0–12.0) | 12.0 (12.0–12.0) | ×1.00 |
| Missed | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |
| Late p50 (ms) | 491 (468–630) | 747 (347–870) | ×1.52 |
| Late max (ms) | 498 (469–631) | 748 (348–870) | ×1.50 |
