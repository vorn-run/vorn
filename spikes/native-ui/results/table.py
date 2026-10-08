"""Per-candidate table for REPORT.md from results/raw/*.json (run by hand)."""
import json, glob, os, statistics

raw = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'raw')
S = {}
for p in glob.glob(os.path.join(raw, '*.json')):
    d = json.load(open(p))
    S[d['name']] = d['summary']

def g(name, k):
    v = S.get(name, {}).get(k)
    return '' if v is None else v

rows = [
    ('Latency p50 / p95, 1 pane (ms)', lambda c: f"{g(c+'-latency-1','latency_p50_ms')} / {g(c+'-latency-1','latency_p95_ms')}"),
    ('Probes lost (of 150), 1 pane', lambda c: g(c+'-latency-1', 'latency_lost')),
    ('Latency p50 / p95, 8 panes, 7 running yes (ms)', lambda c: f"{g(c+'-latency-8-yes','latency_p50_ms')} / {g(c+'-latency-8-yes','latency_p95_ms')}"),
    ('Load yes x8: fps / dropped %', lambda c: f"{g(c+'-load-8-yes','fps')} / {g(c+'-load-8-yes','dropped_pct')}"),
    ('Load yes x8: frame work p50 / p95 (ms)', lambda c: f"{g(c+'-load-8-yes','frame_work_p50_ms')} / {g(c+'-load-8-yes','frame_work_p95_ms')}"),
    ('Load buildlog x8: fps / dropped %', lambda c: f"{g(c+'-load-8-buildlog','fps')} / {g(c+'-load-8-buildlog','dropped_pct')}"),
    ('Load buildlog x8: frame work p50 / p95 (ms)', lambda c: f"{g(c+'-load-8-buildlog','frame_work_p50_ms')} / {g(c+'-load-8-buildlog','frame_work_p95_ms')}"),
    ('Replay vim x8: fps / interval p95 (ms)', lambda c: f"{g(c+'-load-8-rec_vim','fps')} / {g(c+'-load-8-rec_vim','frame_interval_p95_ms')}"),
    ('Client CPU %: idle 8 / yes x8 / buildlog x8', lambda c: f"{g(c+'-idle-8','client_cpu_pct')} / {g(c+'-load-8-yes','client_cpu_pct')} / {g(c+'-load-8-buildlog','client_cpu_pct')}"),
    ('Client footprint MB: idle 8 / yes x8', lambda c: f"{g(c+'-idle-8','client_footprint_max_mb')} / {g(c+'-load-8-yes','client_footprint_max_mb')}"),
    ('Client RSS MB: idle 8 / yes x8', lambda c: f"{g(c+'-idle-8','client_rss_max_mb')} / {g(c+'-load-8-yes','client_rss_max_mb')}"),
    ('vornd CPU %: yes x8 / buildlog x8', lambda c: f"{g(c+'-load-8-yes','vornd_cpu_pct')} / {g(c+'-load-8-buildlog','vornd_cpu_pct')}"),
    ('Cold start to first frame, median of 5 (ms)', lambda c: statistics.median([S[f'{c}-start-1-r{i}']['cold_start_ms'] for i in range(1, 6)])),
]
def pair(run, k1, k2):
    return lambda c: f"{g(c+'-'+run, k1)} / {g(c+'-'+run, k2)}"

# The gated 16/32-pane stress runs: python3 results/table.py --stress apple-a apple-b
stress_rows = [
    ('Latency p50 / p95, 32 panes, 31 running yes (ms)', pair('latency-32-yes', 'latency_p50_ms', 'latency_p95_ms')),
    ('Probes lost (of 150), 32 panes', lambda c: g(c+'-latency-32-yes', 'latency_lost')),
]
for n in (16, 32):
    for p in ('yes', 'buildlog'):
        r = f'load-{n}-{p}'
        stress_rows += [
            (f'Load {p} x{n}: fps / dropped % / interval max (ms)', lambda c, r=r: f"{g(c+'-'+r,'fps')} / {g(c+'-'+r,'dropped_pct')} / {g(c+'-'+r,'frame_interval_max_ms')}"),
            (f'Load {p} x{n}: frame work p50 / p95 (ms)', pair(r, 'frame_work_p50_ms', 'frame_work_p95_ms')),
            (f'Load {p} x{n}: client CPU % / footprint MB', pair(r, 'client_cpu_pct', 'client_footprint_max_mb')),
            (f'Load {p} x{n}: vornd CPU %', lambda c, r=r: g(c+'-'+r, 'vornd_cpu_pct')),
        ]
stress_rows.append(('Idle 32: client CPU % / footprint MB', pair('idle-32', 'client_cpu_pct', 'client_footprint_max_mb')))

import sys
if sys.argv[1:2] == ['--stress']:
    rows = stress_rows
    sys.argv.pop(1)
clients = sys.argv[1:] or ['swift', 'gpui', 'tauri', 'tauri120', 'slint']
print('| metric | ' + ' | '.join(clients) + ' |')
print('|---|' + '---|' * len(clients))
for label, f in rows:
    print(f'| {label} | ' + ' | '.join(str(f(c)) for c in clients) + ' |')
