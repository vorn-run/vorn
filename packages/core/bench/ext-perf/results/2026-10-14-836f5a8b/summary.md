# Tasks and Workflows as extensions: measurements

Machine: {"os":"macos","arch":"aarch64","cpu":"Apple M2 Pro","cores":10,"memory_gb":17.0}. Cells: median of the repeated runs (lowest–highest). Rows left out (failed, or a build ran during them): 13.
Components: {"ext_perf_tasks.wasm":{"wasm_kb":172,"cwasm_kb":468,"compile_ms":127.240958},"ext_perf_workflows.wasm":{"wasm_kb":1668,"cwasm_kb":3751,"compile_ms":1271.3183749999998}}

**Board frames, 100 tasks**

| Metric | builtin | kit | kit-window | kit-window / builtin |
| --- | ---: | ---: | ---: | ---: |
| load ms (read + first frame) | 63.4 (60.5–65.7) | 31.8 (30.0–42.5) | 12.8 (10.8–13.7) | ×0.20 |
| idle frame ms p50 | 27.7 (27.6–29.6) | 6.89 (6.52–7.13) | 5.66 (4.90–6.49) | ×0.20 |
| scroll frame ms p50 | 27.9 (27.7–29.6) | 6.93 (6.82–7.11) | 6.68 (6.27–7.02) | ×0.24 |
| scroll frame ms p99 | 29.8 (28.5–30.3) | 7.57 (7.52–7.84) | 8.19 (7.20–10.2) | ×0.27 |
| scroll frame ms max | 30.3 (28.6–30.6) | 8.10 (7.56–8.41) | 8.47 (8.30–11.0) | ×0.28 |
| drag frame ms p50 | 28.0 (27.9–29.6) | 6.95 (6.89–7.17) | 6.19 (5.42–6.81) | ×0.22 |
| drag frame ms p99 | 38.5 (38.2–40.2) | 9.05 (8.82–9.69) | 7.99 (7.21–8.74) | ×0.21 |
| drag frame ms max | 38.5 (38.2–40.2) | 9.05 (8.82–9.69) | 7.99 (7.21–8.74) | ×0.21 |
| frames timed | 234 (234–234) | 234 (234–234) | 234 (234–234) | ×1.00 |
| frames over 16.7 ms, % | 100 (100–100) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | ×0.00 |
| app footprint MB taken by the board | 76.2 (75.8–77.8) | 76.2 (75.5–77.9) | 72.2 (72.0–73.1) | ×0.95 |

**Board frames, 1000 tasks**

| Metric | builtin | kit | kit-window | kit-window / builtin |
| --- | ---: | ---: | ---: | ---: |
| load ms (read + first frame) | 572 (563–591) | 295 (284–311) | 23.2 (22.9–29.9) | ×0.04 |
| idle frame ms p50 | 278 (272–287) | 82.5 (81.6–88.3) | 5.38 (4.59–5.87) | ×0.02 |
| scroll frame ms p50 | 277 (273–289) | 82.8 (81.9–90.5) | 6.63 (5.65–6.86) | ×0.02 |
| scroll frame ms p99 | 294 (275–304) | 90.3 (86.8–126) | 8.38 (7.84–9.97) | ×0.03 |
| scroll frame ms max | 294 (275–304) | 92.4 (87.3–232) | 8.62 (7.91–10.4) | ×0.03 |
| drag frame ms p50 | 293 (279–295) | 86.1 (84.9–93.3) | 6.68 (5.11–6.93) | ×0.02 |
| drag frame ms p99 | 396 (389–549) | 134 (129–231) | 7.95 (7.53–10.6) | ×0.02 |
| drag frame ms max | 396 (389–549) | 134 (129–231) | 7.95 (7.53–10.6) | ×0.02 |
| frames timed | 42.0 (42.0–42.0) | 234 (234–234) | 234 (234–234) | ×5.57 |
| frames over 16.7 ms, % | 100 (100–100) | 100 (100–100) | 0.00 (0.00–0.00) | ×0.00 |
| app footprint MB taken by the board | 220 (218–222) | 215 (211–216) | 77.2 (77.2–79.4) | ×0.35 |

**Board frames, 10000 tasks**

| Metric | builtin | kit | kit-window | kit-window / builtin |
| --- | ---: | ---: | ---: | ---: |
| load ms (read + first frame) | 6064 (6025–6337) | 3191 (3158–3339) | 120 (119–128) | ×0.02 |
| idle frame ms p50 | 3055 (2989–3124) | 1024 (1014–1027) | 6.38 (5.68–6.60) | ×0.00 |
| scroll frame ms p50 | 3059 (3021–3084) | 1037 (1018–1047) | 6.74 (6.04–6.94) | ×0.00 |
| scroll frame ms p99 | 3150 (3107–3220) | 1094 (1048–1227) | 8.26 (7.86–11.2) | ×0.00 |
| scroll frame ms max | 3150 (3107–3220) | 1094 (1048–1227) | 8.89 (8.05–12.2) | ×0.00 |
| drag frame ms p50 | 3104 (3027–3227) | 1037 (1023–1119) | 6.42 (6.22–6.60) | ×0.00 |
| drag frame ms p99 | 3457 (3370–3541) | 1744 (1710–1939) | 8.17 (7.98–33.4) | ×0.00 |
| drag frame ms max | 3457 (3370–3541) | 1744 (1710–1939) | 8.17 (7.98–33.4) | ×0.00 |
| frames timed | 42.0 (42.0–42.0) | 42.0 (42.0–42.0) | 234 (234–234) | ×5.57 |
| frames over 16.7 ms, % | 100 (100–100) | 100 (100–100) | 0.00 (0.00–3.42) | ×0.00 |
| app footprint MB taken by the board | 1806 (1529–2068) | 1693 (1687–1964) | 121 (120–122) | ×0.07 |

**Workflows: 1 concurrent runs of 10 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 3.15 (2.04–3.66) | 1.88 (0.36–2.27) | 2.12 (1.65–3.12) | ×0.67 |
| trigger → first step ms p99 | 4.48 (3.71–4.78) | 4.45 (0.75–8.94) | 3.76 (2.84–9.70) | ×0.84 |
| first step seen by a viewer ms p50 | 7.54 (7.09–8.06) | 2.50 (0.89–2.93) | 3.27 (2.56–4.48) | ×0.43 |
| first step seen by a viewer ms p99 | 12.1 (10.0–12.6) | 5.97 (1.15–11.2) | 5.25 (4.10–10.3) | ×0.43 |
| wall µs per step p50 | 300 (300–400) | 300 (100–300) | 500 (400–700) | ×1.67 |
| wall µs per step p99 | 700 (500–800) | 600 (200–700) | 1000 (700–1200) | ×1.43 |
| vornd CPU µs per step | 1249 (1128–1418) | 905 (394–978) | 1327 (1211–1558) | ×1.06 |
| steps per second | 1132 (992–1185) | 1569 (1499–4874) | 1077 (691–1349) | ×0.95 |
| peak KB per run | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |
| run broadcasts MB per case | 0.23 (0.23–0.23) | 0.23 (0.23–0.23) | 0.23 (0.23–0.23) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Workflows: 10 concurrent runs of 10 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 14.9 (9.18–20.4) | 5.47 (2.60–7.50) | 5.91 (3.21–8.52) | ×0.40 |
| trigger → first step ms p99 | 33.4 (25.5–36.1) | 11.7 (11.5–18.7) | 16.3 (11.4–21.1) | ×0.49 |
| first step seen by a viewer ms p50 | 25.0 (11.7–30.7) | 8.67 (3.91–10.6) | 12.9 (4.59–19.6) | ×0.52 |
| first step seen by a viewer ms p99 | 35.8 (27.4–41.2) | 15.6 (14.3–20.9) | 21.8 (18.1–24.4) | ×0.61 |
| wall µs per step p50 | 500 (300–700) | 1100 (900–1400) | 1300 (1000–1400) | ×2.60 |
| wall µs per step p99 | 1300 (500–1700) | 1900 (1600–2100) | 2100 (1800–2500) | ×1.62 |
| vornd CPU µs per step | 1039 (524–1139) | 740 (549–811) | 896 (793–1081) | ×0.86 |
| steps per second | 2725 (2486–6717) | 3892 (3655–6674) | 4275 (3399–7067) | ×1.57 |
| peak KB per run | 11.2 (0.00–28.8) | 12.8 (0.00–33.6) | 422 (0.00–792) | ×37.72 |
| run broadcasts MB per case | 0.70 (0.70–0.70) | 0.70 (0.70–0.70) | 0.70 (0.70–0.70) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Workflows: 100 concurrent runs of 10 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 353 (330–373) | 32.2 (13.2–42.4) | 38.7 (17.0–44.2) | ×0.11 |
| trigger → first step ms p99 | 643 (628–672) | 49.7 (32.2–54.0) | 57.7 (30.0–59.0) | ×0.09 |
| first step seen by a viewer ms p50 | 355 (331–374) | 33.5 (15.6–45.4) | 41.6 (19.3–52.4) | ×0.12 |
| first step seen by a viewer ms p99 | 644 (631–675) | 50.5 (33.1–64.8) | 58.7 (50.0–59.5) | ×0.09 |
| wall µs per step p50 | 100 (100–200) | 4400 (3700–4900) | 5000 (3600–5400) | ×50.00 |
| wall µs per step p99 | 200 (200–500) | 5800 (4700–6100) | 6600 (4700–7900) | ×33.00 |
| vornd CPU µs per step | 1232 (1220–1287) | 473 (403–510) | 596 (458–651) | ×0.48 |
| steps per second | 1534 (1464–1567) | 8369 (3738–8816) | 7366 (7009–9281) | ×4.80 |
| peak KB per run | 958 (952–990) | 897 (889–927) | 1327 (1302–1451) | ×1.38 |
| run broadcasts MB per case | 2.33 (2.33–2.33) | 2.33 (2.33–2.33) | 2.33 (2.33–2.33) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Workflows: 1 concurrent runs of 100 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 6.12 (3.57–8.30) | 2.71 (2.59–3.17) | 4.96 (3.91–6.36) | ×0.81 |
| trigger → first step ms p99 | 8.64 (5.70–11.3) | 4.35 (3.92–5.22) | 7.12 (5.52–10.7) | ×0.82 |
| first step seen by a viewer ms p50 | 96.3 (90.8–109) | 4.07 (3.36–4.45) | 6.34 (5.20–8.10) | ×0.07 |
| first step seen by a viewer ms p99 | 117 (98.0–123) | 5.33 (4.79–5.95) | 8.97 (6.87–12.6) | ×0.08 |
| wall µs per step p50 | 890 (860–1000) | 730 (680–770) | 1130 (1020–1180) | ×1.27 |
| wall µs per step p99 | 1050 (910–1150) | 800 (700–850) | 1180 (1050–1200) | ×1.12 |
| vornd CPU µs per step | 1817 (1645–2050) | 1441 (1293–1515) | 2205 (1865–2350) | ×1.21 |
| steps per second | 927 (855–1022) | 1305 (1236–1386) | 830 (792–938) | ×0.90 |
| peak KB per run | 16.0 (0.00–208) | 32.0 (0.00–128) | 1600 (1376–1632) | ×100.00 |
| run broadcasts MB per case | 16.7 (16.7–16.7) | 16.7 (16.7–16.7) | 16.7 (16.7–16.7) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Workflows: 10 concurrent runs of 100 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 50.3 (49.7–54.5) | 9.72 (9.06–10.3) | 10.7 (10.5–11.5) | ×0.21 |
| trigger → first step ms p99 | 91.3 (87.7–104) | 13.4 (12.0–17.1) | 15.9 (13.7–19.7) | ×0.17 |
| first step seen by a viewer ms p50 | 262 (252–280) | 11.5 (10.5–11.9) | 12.7 (12.1–12.8) | ×0.05 |
| first step seen by a viewer ms p99 | 277 (266–304) | 15.7 (13.9–19.7) | 17.3 (16.5–21.2) | ×0.06 |
| wall µs per step p50 | 2680 (2580–2950) | 2750 (2650–3610) | 3150 (3040–3350) | ×1.18 |
| wall µs per step p99 | 2830 (2770–3100) | 2930 (2760–3790) | 3470 (3180–3570) | ×1.23 |
| vornd CPU µs per step | 1685 (1639–1726) | 1578 (1492–1711) | 2209 (2080–2226) | ×1.31 |
| steps per second | 2790 (2591–2884) | 3409 (2595–3552) | 2962 (2797–3056) | ×1.06 |
| peak KB per run | 58078 (56822–60203) | 53058 (52008–55046) | 55838 (55331–58381) | ×0.96 |
| run broadcasts MB per case | 50.1 (50.1–50.1) | 50.1 (50.1–50.1) | 50.1 (50.1–50.1) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Workflows: 100 concurrent runs of 100 steps**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → first step ms p50 | 3224 (2963–3571) | 86.4 (80.0–125) | 119 (91.7–128) | ×0.04 |
| trigger → first step ms p99 | 6509 (6141–6865) | 142 (120–162) | 192 (166–212) | ×0.03 |
| first step seen by a viewer ms p50 | 3305 (2980–3595) | 99.3 (91.0–135) | 132 (108–148) | ×0.04 |
| first step seen by a viewer ms p99 | 6522 (6155–6885) | 155 (134–174) | 211 (185–255) | ×0.03 |
| wall µs per step p50 | 1350 (1280–1380) | 30850 (26140–33010) | 33450 (28270–36010) | ×24.78 |
| wall µs per step p99 | 1860 (1500–3240) | 32110 (27420–34000) | 34970 (29890–37940) | ×18.80 |
| vornd CPU µs per step | 2446 (2315–2503) | 1608 (1463–1796) | 2047 (1911–2351) | ×0.84 |
| steps per second | 1491 (1409–1580) | 3038 (2840–3523) | 2774 (2551–3221) | ×1.86 |
| peak KB per run | 68950 (68724–69434) | 64549 (63955–65852) | 66487 (66419–66897) | ×0.96 |
| run broadcasts MB per case | 167 (167–167) | 167 (167–167) | 167 (167–167) | ×1.00 |
| failed runs | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | – |

**Gates: 1 runs parked at once**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → gate waiting ms p50 | 3.33 (3.09–4.01) | 2.34 (1.51–2.86) | 3.83 (3.46–4.06) | ×1.15 |
| resolveGate answer ms p50 | 2.08 (1.72–2.26) | 1.82 (1.12–2.49) | 1.97 (1.81–2.12) | ×0.94 |
| resolveGate answer ms p99 | 3.22 (2.68–3.53) | 3.55 (2.86–13.1) | 2.92 (2.59–4.46) | ×0.91 |
| answer → next step starts ms p50 | 1.68 (1.50–2.13) | 1.66 (0.89–2.02) | 2.30 (1.82–2.60) | ×1.37 |
| answer → next step starts ms p99 | 4.06 (2.55–4.79) | 3.25 (2.55–4.44) | 3.98 (3.04–4.89) | ×0.98 |
| answer → viewer sees it ms p99 | 4.24 (3.27–5.90) | 3.92 (3.00–13.1) | 4.82 (3.83–6.01) | ×1.14 |
| parked KB per run | 0.00 (0.00–0.00) | 0.00 (0.00–0.00) | 0.00 (0.00–16.0) | – |

**Gates: 10 runs parked at once**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → gate waiting ms p50 | 12.9 (11.0–15.1) | 6.03 (4.63–10.2) | 9.87 (9.20–11.0) | ×0.77 |
| resolveGate answer ms p50 | 2.96 (2.45–4.75) | 2.59 (1.07–3.23) | 1.83 (1.47–2.94) | ×0.62 |
| resolveGate answer ms p99 | 5.88 (5.35–7.78) | 5.48 (1.29–6.33) | 4.16 (2.06–4.62) | ×0.71 |
| answer → next step starts ms p50 | 2.69 (2.11–3.97) | 2.09 (0.29–2.81) | 1.52 (1.18–3.03) | ×0.57 |
| answer → next step starts ms p99 | 5.99 (5.09–7.77) | 4.78 (1.04–5.18) | 4.12 (1.62–4.73) | ×0.69 |
| answer → viewer sees it ms p99 | 6.18 (5.52–7.91) | 5.54 (1.30–6.35) | 4.71 (2.19–5.07) | ×0.76 |
| parked KB per run | 96.0 (89.6–101) | 112 (110–122) | 139 (138–149) | ×1.45 |

**Gates: 100 runs parked at once**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| trigger → gate waiting ms p50 | 161 (139–168) | 25.1 (14.6–39.4) | 33.8 (29.5–42.6) | ×0.21 |
| resolveGate answer ms p50 | 1.10 (1.04–1.27) | 0.74 (0.71–0.84) | 0.73 (0.68–0.83) | ×0.66 |
| resolveGate answer ms p99 | 2.78 (1.89–6.03) | 1.66 (1.58–1.91) | 1.66 (1.41–6.59) | ×0.60 |
| answer → next step starts ms p50 | 0.69 (0.64–0.82) | 0.22 (0.15–0.28) | 0.36 (0.34–0.41) | ×0.53 |
| answer → next step starts ms p99 | 2.42 (1.53–5.38) | 1.20 (0.98–1.68) | 1.56 (1.12–5.79) | ×0.65 |
| answer → viewer sees it ms p99 | 2.84 (2.02–6.08) | 1.67 (1.59–1.92) | 1.76 (1.51–6.67) | ×0.62 |
| parked KB per run | 93.9 (92.6–96.5) | 109 (101–114) | 117 (112–139) | ×1.25 |

**In process, Tasks calls at 100 tasks (µs, p50 and p99 of each round)**

| Metric | builtin | native-json | component | component / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list p50 | 597 (305–881) | 918 (479–1387) | 1387 (729–2100) | ×2.32 |
| task:create p50 | 93.4 (86.0–186) | 99.9 (92.2–188) | 109 (100–130) | ×1.17 |
| task:create p99 | 163 (112–1693) | 156 (123–3934) | 171 (132–2314) | ×1.05 |
| task:update p50 | 62.9 (59.0–93.2) | 72.1 (67.2–121) | 77.7 (72.6–98.5) | ×1.24 |
| task:update p99 | 83.3 (75.1–274) | 96.0 (84.5–334) | 106 (88.0–503) | ×1.27 |
| task:setStatus p50 | 51.3 (47.2–98.5) | 58.0 (52.0–115) | 58.9 (55.6–98.4) | ×1.15 |
| task:setStatus p99 | 69.1 (59.9–1600) | 107 (63.1–982) | 87.5 (65.5–683) | ×1.27 |

**In process, the step loop at 100 tasks (no socket, no store)**

| Chain | native µs/step p50 | component µs/step p50 | component µs/step p99 | instantiate µs p50 | instance KB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 steps | 28.2 (28.1–29.9) | 82.0 (80.6–88.6) | 144 (125–224) | 45.1 (42.8–54.8) | 1728 (1728–1728) |
| 100 steps | 208 (207–213) | 537 (534–552) | 547 (543–708) | 66.9 (54.6–101) | 2368 (2368–2368) |

**In process, Tasks calls at 1000 tasks (µs, p50 and p99 of each round)**

| Metric | builtin | native-json | component | component / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list p50 | 3370 (3015–3880) | 5114 (4673–5795) | 7774 (7040–8776) | ×2.31 |
| task:create p50 | 167 (154–179) | 172 (159–191) | 179 (164–199) | ×1.07 |
| task:create p99 | 239 (194–281) | 241 (203–379) | 265 (205–889) | ×1.11 |
| task:update p50 | 64.1 (61.1–74.8) | 70.4 (67.2–78.9) | 77.7 (76.0–83.4) | ×1.21 |
| task:update p99 | 93.8 (78.5–200) | 101 (87.2–207) | 116 (95.4–611) | ×1.23 |
| task:setStatus p50 | 53.5 (49.0–78.4) | 58.3 (52.3–81.4) | 64.2 (57.8–83.1) | ×1.20 |
| task:setStatus p99 | 107 (62.0–330) | 79.4 (66.5–182) | 106 (75.8–563) | ×1.00 |

**In process, the step loop at 1000 tasks (no socket, no store)**

| Chain | native µs/step p50 | component µs/step p50 | component µs/step p99 | instantiate µs p50 | instance KB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 steps | 29.2 (27.5–29.7) | 83.7 (81.3–86.5) | 130 (127–141) | 46.5 (43.3–53.8) | 1728 (1728–1728) |
| 100 steps | 209 (208–216) | 542 (536–559) | 591 (548–605) | 71.0 (63.0–99.4) | 2368 (2368–2368) |

**In process, Tasks calls at 10000 tasks (µs, p50 and p99 of each round)**

| Metric | builtin | native-json | component | component / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list p50 | 32067 (30928–33766) | 49382 (47658–51495) | 73392 (72044–75306) | ×2.29 |
| task:create p50 | 876 (852–1003) | 881 (858–930) | 892 (858–923) | ×1.02 |
| task:create p99 | 1149 (924–1390) | 1133 (944–1390) | 975 (928–1427) | ×0.85 |
| task:update p50 | 66.1 (63.5–89.5) | 75.9 (70.8–86.5) | 82.4 (77.9–94.0) | ×1.25 |
| task:update p99 | 126 (88.4–198) | 146 (107–255) | 126 (111–171) | ×1.01 |
| task:setStatus p50 | 54.1 (51.3–86.0) | 63.4 (55.5–87.7) | 64.2 (59.1–93.7) | ×1.19 |
| task:setStatus p99 | 109 (68.5–503) | 134 (90.8–348) | 118 (83.2–183) | ×1.09 |

**In process, the step loop at 10000 tasks (no socket, no store)**

| Chain | native µs/step p50 | component µs/step p50 | component µs/step p99 | instantiate µs p50 | instance KB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 steps | 28.5 (27.9–29.1) | 83.3 (81.8–84.6) | 139 (130–151) | 50.8 (44.2–54.7) | 1728 (1728–1728) |
| 100 steps | 212 (208–213) | 543 (535–556) | 560 (548–604) | 69.4 (57.3–90.9) | 2368 (2368–2368) |

**Schedules: 12 workflows due at their own second**

| Metric | builtin | native | ext | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| start after the earliest of its case ms p99 (jitter) | 8.00 (5.00–16.0) | 4.00 (4.00–7.00) | 6.00 (3.00–6.00) | ×0.75 |
| seen by a viewer late ms p50 | 372 (129–712) | 851 (24.9–1002) | 509 (234–850) | ×1.37 |
| start late ms p50 | 370 (128–711) | 849 (24.0–1001) | 508 (233–849) | ×1.37 |
| start late ms p99 | 374 (131–716) | 850 (25.0–1004) | 510 (235–850) | ×1.36 |
| start late ms max | 374 (131–716) | 850 (25.0–1004) | 510 (235–850) | ×1.36 |
| fired of 12 | 12.0 (12.0–12.0) | 12.0 (12.0–12.0) | 12.0 (12.0–12.0) | ×1.00 |

**Tasks against a test vornd, 100 tasks**

| Metric | builtin | ext | ext-config | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list ms p50 | 0.90 (0.78–1.06) | 1.26 (1.17–1.65) | 1.31 (1.20–1.47) | ×1.40 |
| task:list ms p99 | 0.98 (0.91–1.28) | 1.82 (1.40–4.94) | 1.62 (1.25–2.08) | ×1.85 |
| config:load ms p50 (today's board read) | 0.89 (0.85–1.01) | 0.89 (0.85–1.12) | 0.91 (0.86–1.11) | ×1.00 |
| create answer ms p50 | 3.26 (3.24–3.41) | 2.53 (2.19–3.67) | 3.30 (3.23–3.33) | ×0.78 |
| create answer ms p99 | 12.3 (9.03–14.1) | 21.5 (5.87–57.8) | 15.2 (7.71–19.1) | ×1.74 |
| create settle ms p99 | 12.3 (9.03–14.1) | 21.5 (5.87–57.8) | 15.2 (7.71–19.1) | ×1.74 |
| update answer ms p50 | 3.33 (3.18–3.42) | 3.41 (2.12–4.84) | 3.35 (3.31–3.41) | ×1.02 |
| update answer ms p99 | 14.5 (9.47–14.6) | 19.8 (6.91–26.9) | 12.0 (6.68–14.8) | ×1.37 |
| move answer ms p50 | 3.52 (3.29–3.77) | 2.13 (2.04–3.43) | 3.44 (3.30–4.25) | ×0.60 |
| move answer ms p99 | 12.2 (7.66–15.5) | 17.0 (5.69–22.9) | 13.4 (6.86–16.4) | ×1.40 |
| move settle ms p99 | 12.2 (7.66–15.5) | 17.0 (5.69–22.9) | 13.4 (6.86–16.4) | ×1.40 |
| change broadcast bytes p50 | 37974 (37973–37974) | 101 (101–101) | 37974 (37974–37974) | ×0.00 |
| board drop via config:save ms p50 | 5.39 (5.27–5.58) | – | – | – |
| board drop via config:save ms p99 | 6.68 (6.32–7.75) | – | – | – |
| vornd footprint MB at end | 11.0 (10.6–11.2) | 9.53 (9.52–9.88) | 10.3 (10.1–10.6) | ×0.87 |

**Tasks against a test vornd, 1000 tasks**

| Metric | builtin | ext | ext-config | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list ms p50 | 4.57 (4.52–5.02) | 8.58 (8.40–8.78) | 8.63 (8.40–8.87) | ×1.88 |
| task:list ms p99 | 4.74 (4.58–5.63) | 9.04 (8.50–11.7) | 8.87 (8.49–9.45) | ×1.91 |
| config:load ms p50 (today's board read) | 4.50 (4.36–4.70) | 4.50 (4.39–4.52) | 4.41 (4.38–4.50) | ×1.00 |
| create answer ms p50 | 7.22 (7.12–8.79) | 2.71 (2.23–8.16) | 7.32 (7.25–7.37) | ×0.37 |
| create answer ms p99 | 19.9 (12.5–98.3) | 39.6 (7.42–143) | 17.7 (11.6–22.7) | ×1.99 |
| create settle ms p99 | 19.9 (12.5–98.3) | 39.6 (7.42–143) | 17.7 (11.6–22.7) | ×1.99 |
| update answer ms p50 | 7.21 (7.14–7.88) | 2.25 (1.92–3.73) | 7.23 (7.19–7.28) | ×0.31 |
| update answer ms p99 | 13.9 (11.0–15.3) | 18.0 (4.80–24.1) | 12.8 (10.2–15.8) | ×1.30 |
| move answer ms p50 | 7.22 (7.09–7.55) | 3.10 (1.97–3.35) | 7.24 (7.20–7.34) | ×0.43 |
| move answer ms p99 | 11.7 (8.67–36.2) | 20.8 (7.27–32.6) | 13.4 (11.2–15.0) | ×1.78 |
| move settle ms p99 | 11.7 (8.67–36.2) | 20.8 (7.27–32.6) | 13.4 (11.2–15.0) | ×1.78 |
| change broadcast bytes p50 | 244233 (244232–244233) | 101 (101–101) | 244233 (244233–244233) | ×0.00 |
| board drop via config:save ms p50 | 33.5 (33.2–34.7) | – | – | – |
| board drop via config:save ms p99 | 42.7 (38.5–45.0) | – | – | – |
| vornd footprint MB at end | 32.3 (31.3–33.8) | 17.4 (16.8–17.9) | 21.7 (21.2–21.8) | ×0.54 |

**Tasks against a test vornd, 10000 tasks**

| Metric | builtin | ext | ext-config | ext / builtin |
| --- | ---: | ---: | ---: | ---: |
| task:list ms p50 | 43.8 (43.2–45.3) | 83.2 (82.1–85.6) | 83.3 (82.3–83.9) | ×1.90 |
| task:list ms p99 | 45.0 (44.2–46.5) | 85.1 (84.6–110) | 85.3 (83.4–88.2) | ×1.89 |
| config:load ms p50 (today's board read) | 41.1 (40.7–42.3) | 41.3 (40.8–41.8) | 41.2 (40.8–42.8) | ×1.01 |
| create answer ms p50 | 52.8 (52.4–54.8) | 4.48 (3.06–4.95) | 52.9 (52.7–53.9) | ×0.08 |
| create answer ms p99 | 61.5 (55.3–147) | 22.1 (9.82–43.4) | 59.6 (56.2–81.2) | ×0.36 |
| create settle ms p99 | 61.5 (55.3–147) | 22.1 (9.82–43.4) | 59.6 (56.2–81.2) | ×0.36 |
| update answer ms p50 | 51.5 (51.2–53.3) | 1.98 (1.87–2.91) | 51.4 (51.2–52.2) | ×0.04 |
| update answer ms p99 | 60.6 (56.0–67.5) | 17.3 (3.77–29.8) | 57.0 (53.6–61.1) | ×0.29 |
| move answer ms p50 | 52.3 (51.1–52.6) | 2.89 (1.85–3.46) | 51.5 (50.8–52.6) | ×0.06 |
| move answer ms p99 | 60.2 (52.7–69.9) | 18.0 (4.64–18.9) | 65.8 (53.0–84.0) | ×0.30 |
| move settle ms p99 | 60.2 (52.7–69.9) | 18.0 (4.64–18.9) | 65.8 (53.0–84.0) | ×0.30 |
| change broadcast bytes p50 | 2324267 (2324267–2324268) | 101 (101–101) | 2324268 (2324267–2324268) | ×0.00 |
| board drop via config:save ms p50 | 310 (304–311) | – | – | – |
| board drop via config:save ms p99 | 328 (325–339) | – | – | – |
| vornd footprint MB at end | 349 (334–499) | 261 (257–262) | 325 (320–326) | ×0.75 |

**Decision rule: extension within ~10% of built-in (p99 and memory)**

| Check | ext / builtin | Within 10% |
| --- | ---: | :---: |
| Workflows 10×1: trigger→first p99 | ×0.84 | yes |
| Workflows 10×1: per step p99 | ×1.43 | no |
| Workflows 10×1: CPU per step | ×1.06 | yes |
| Workflows 10×1: time per step at throughput (builtin steps/s ÷ ext) | ×1.05 | yes |
| Workflows 10×10: trigger→first p99 | ×0.49 | yes |
| Workflows 10×10: CPU per step | ×0.86 | yes |
| Workflows 10×10: peak KB/run | ×37.72 | no |
| Workflows 10×10: time per step at throughput (builtin steps/s ÷ ext) | ×0.64 | yes |
| Workflows 10×100: trigger→first p99 | ×0.09 | yes |
| Workflows 10×100: CPU per step | ×0.48 | yes |
| Workflows 10×100: peak KB/run | ×1.38 | no |
| Workflows 10×100: time per step at throughput (builtin steps/s ÷ ext) | ×0.21 | yes |
| Workflows 100×1: trigger→first p99 | ×0.82 | yes |
| Workflows 100×1: per step p99 | ×1.12 | no |
| Workflows 100×1: CPU per step | ×1.21 | no |
| Workflows 100×1: peak KB/run | ×100.00 | no |
| Workflows 100×1: time per step at throughput (builtin steps/s ÷ ext) | ×1.12 | no |
| Workflows 100×10: trigger→first p99 | ×0.17 | yes |
| Workflows 100×10: CPU per step | ×1.31 | no |
| Workflows 100×10: peak KB/run | ×0.96 | yes |
| Workflows 100×10: time per step at throughput (builtin steps/s ÷ ext) | ×0.94 | yes |
| Workflows 100×100: trigger→first p99 | ×0.03 | yes |
| Workflows 100×100: CPU per step | ×0.84 | yes |
| Workflows 100×100: peak KB/run | ×0.96 | yes |
| Workflows 100×100: time per step at throughput (builtin steps/s ÷ ext) | ×0.54 | yes |
| Gates 1: answer p99 | ×0.91 | yes |
| Gates 1: resume p99 | ×0.98 | yes |
| Gates 10: answer p99 | ×0.71 | yes |
| Gates 10: resume p99 | ×0.69 | yes |
| Gates 100: answer p99 | ×0.60 | yes |
| Gates 100: resume p99 | ×0.65 | yes |
| Schedule: jitter p99 | ×0.75 | yes |
| Tasks 100: create p99 (rows) | ×1.74 | no |
| Tasks 100: create p99 (config broadcast) | ×1.24 | no |
| Tasks 100: update p99 (rows) | ×1.37 | no |
| Tasks 100: update p99 (config broadcast) | ×0.83 | yes |
| Tasks 100: move p99 (rows) | ×1.40 | no |
| Tasks 100: move p99 (config broadcast) | ×1.10 | no |
| Tasks 100: list p99 (rows) | ×1.85 | no |
| Tasks 100: list p99 (config broadcast) | ×1.65 | no |
| Tasks 100: vornd MB (rows) | ×0.87 | yes |
| Tasks 100: vornd MB (config broadcast) | ×0.94 | yes |
| Tasks 1000: create p99 (rows) | ×1.99 | no |
| Tasks 1000: create p99 (config broadcast) | ×0.89 | yes |
| Tasks 1000: update p99 (rows) | ×1.30 | no |
| Tasks 1000: update p99 (config broadcast) | ×0.92 | yes |
| Tasks 1000: move p99 (rows) | ×1.78 | no |
| Tasks 1000: move p99 (config broadcast) | ×1.15 | no |
| Tasks 1000: list p99 (rows) | ×1.91 | no |
| Tasks 1000: list p99 (config broadcast) | ×1.87 | no |
| Tasks 1000: vornd MB (rows) | ×0.54 | yes |
| Tasks 1000: vornd MB (config broadcast) | ×0.67 | yes |
| Tasks 10000: create p99 (rows) | ×0.36 | yes |
| Tasks 10000: create p99 (config broadcast) | ×0.97 | yes |
| Tasks 10000: update p99 (rows) | ×0.29 | yes |
| Tasks 10000: update p99 (config broadcast) | ×0.94 | yes |
| Tasks 10000: move p99 (rows) | ×0.30 | yes |
| Tasks 10000: move p99 (config broadcast) | ×1.09 | yes |
| Tasks 10000: list p99 (rows) | ×1.89 | no |
| Tasks 10000: list p99 (config broadcast) | ×1.90 | no |
| Tasks 10000: vornd MB (rows) | ×0.75 | yes |
| Tasks 10000: vornd MB (config broadcast) | ×0.93 | yes |
