"""Session analytics from MangoHud CSV recordings, never inferred FPS."""
import csv
import math
from pathlib import Path
import statistics
from collections import deque
from .common import DATA, read
from .sessions import list_sessions


def parse_trace(path):
    # Bound memory for long-running sessions. Metrics describe this recent
    # sample window; a full-day trace cannot allocate unbounded Python objects.
    rows = deque(maxlen=100000)
    total = 0
    with Path(path).open(errors='replace') as src:
        header = None
        for line in src:
            values = next(csv.reader([line]))
            names = [v.strip().lower() for v in values]
            if 'fps' in names and ('frametime' in names or 'frame_timing' in names):
                header = names
                continue
            if header and len(values) == len(header):
                row = {}
                for name, value in zip(header, values):
                    try:
                        v = float(value)
                        if math.isfinite(v): row[name] = v
                    except ValueError:
                        pass
                if row.get('fps', 0) > 0:
                    rows.append(row)
                    total += 1
    if not rows:
        return None
    fps = [r['fps'] for r in rows]
    frames = [r.get('frametime', r.get('frame_timing', 1000/r['fps'])) for r in rows]
    sorted_frames = sorted(frames)
    p99 = sorted_frames[min(len(frames)-1, int(len(frames)*.99))]
    median = statistics.median(frames)
    return dict(samples=len(rows), truncated=total>len(rows), avg_fps=round(statistics.mean(fps), 1),
                p99_ms=round(p99, 2), low_fps=round(1000/p99, 1) if p99 > 0 else None,
                stutters=sum(v > max(33.3, median*2) for v in frames),
                gpu_temp=round(statistics.mean([r['gpu_temp'] for r in rows if r.get('gpu_temp',0)>0]), 1) if any(r.get('gpu_temp',0)>0 for r in rows) else None,
                points=[round(v, 2) for v in frames[::max(1, len(frames)//180)]][-180:])


def history(gid=None):
    result = []
    for s in list_sessions():
        if gid and s['game'] != gid: continue
        traces = sorted((DATA / 'traces' / s['id']).glob('*.csv'))
        stats = None
        for f in traces:
            stats = parse_trace(f) or stats
        result.append({**s, 'stats': stats})
    return result


def recommendations(gid):
    sessions = [s for s in history(gid) if s.get('stats')]
    if not sessions:
        return {'summary': 'Record a managed session with MangoHud to analyze frame timing.', 'evidence': [], 'suggestions': []}
    s = sessions[0]['stats']
    suggestions = []
    if s['stutters']:
        suggestions.append(f"The recording contains {s['stutters']} frame-time samples over twice the median or 33.3 ms. Compare a second session after changing one setting.")
    if s.get('gpu_temp', 0) and s['gpu_temp'] > 80:
        suggestions.append('Average recorded GPU temperature exceeded 80 °C. Check cooling and compare a lower power profile before changing graphics settings.')
    return {'summary': f"Latest recording: {s['avg_fps']} FPS average, {s['p99_ms']} ms at the 99th percentile.",
            'evidence': [dict(session=sessions[0]['id'], **s)], 'suggestions': suggestions}


def compare(before, after):
    records = {s['id']: s for s in history()}
    a, b = records.get(before), records.get(after)
    if not a or not b or not a['stats'] or not b['stats'] or a['game'] != b['game']:
        raise ValueError('Choose two recorded sessions of the same game')
    return dict(before=a, after=b, fps_delta=round(b['stats']['avg_fps']-a['stats']['avg_fps'], 1),
                caveat='Measured session difference; scenes, resolution and background activity can affect the result.')
