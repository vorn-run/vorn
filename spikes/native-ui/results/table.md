| metric | swift | gpui | tauri | tauri120 | slint |
|---|---|---|---|---|---|
| Latency p50 / p95, 1 pane (ms) | 5.8 / 9.4 | 4.8 / 8.1 | 2.0 / 7.0 | 4.0 / 6.0 | 8.4 / 10.3 |
| Probes lost (of 150), 1 pane | 63 | 0 | 0 | 0 | 0 |
| Latency p50 / p95, 8 panes, 7 running yes (ms) | 7.0 / 8.8 | 1.3 / 8.1 | 26.0 / 33.0 | 14.0 / 17.0 | 7.9 / 13.0 |
| Load yes x8: fps / dropped % | 120.0 / 0.0 | 119.6 / 0.3 | 60.0 / 50.0 | 120.0 / 0.0 | 119.8 / 0.2 |
| Load yes x8: frame work p50 / p95 (ms) | 2.6 / 2.7 | 1.5 / 1.7 | 1.0 / 2.0 | 1.0 / 1.0 | 4.9 / 5.1 |
| Load buildlog x8: fps / dropped % | 119.8 / 0.2 | 120.0 / 0.0 | 60.0 / 50.0 | 120.0 / 0.0 | 96.9 / 12.5 |
| Load buildlog x8: frame work p50 / p95 (ms) | 4.1 / 4.4 | 2.0 / 2.5 | 2.0 / 3.0 | 2.0 / 2.0 | 7.8 / 8.3 |
| Replay vim x8: fps / interval p95 (ms) | 76.1 / 35.1 | 103.5 / 16.8 | 60.0 / 18.0 | 111.8 / 17.0 | 88.7 / 25.4 |
| Client CPU %: idle 8 / yes x8 / buildlog x8 | 0.4 / 36.5 / 53.7 | 1.7 / 32.4 / 40.2 | 7.7 / 55.0 / 62.4 | 2.2 / 140.0 / 152.6 | 2.9 / 74.4 / 95.9 |
| Client footprint MB: idle 8 / yes x8 | 23.9 / 34.4 | 71.3 / 106.3 | 113.3 / 218.4 | 111.1 / 429.4 | 148.7 / 179.1 |
| Client RSS MB: idle 8 / yes x8 | 93.5 / 99.0 | 90.5 / 88.4 | 172.1 / 232.0 | 171.8 / 347.6 | 100.9 / 109.1 |
| vornd CPU %: yes x8 / buildlog x8 | 65.6 / 27.9 | 67.8 / 27.3 | 55.8 / 26.2 | 63.7 / 25.6 | 55.6 / 23.5 |
| Cold start to first frame, median of 5 (ms) | 204.7 | 164.1 | 513.3 | 516.2 | 449.3 |
