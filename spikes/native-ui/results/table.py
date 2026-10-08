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
import sys
clients = sys.argv[1:] or ['swift', 'gpui', 'tauri', 'tauri120', 'slint']
print('| metric | ' + ' | '.join(clients) + ' |')
print('|---|' + '---|' * len(clients))
for label, f in rows:
    print(f'| {label} | ' + ' | '.join(str(f(c)) for c in clients) + ' |')
