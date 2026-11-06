#!/usr/bin/env python3
"""Project Blender sparse frame timings; never label a projection as a measured movie."""
import argparse
import json
from pathlib import Path
import statistics
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('root',type=Path)
a=p.parse_args()
shot=json.loads((a.root/'shot.json').read_text())
groups={}
for path in sorted((a.root/'results').glob('*-events.jsonl')):
    for line in path.read_text().splitlines():
        row=json.loads(line)
        if row.get('kind')=='frame':
            groups.setdefault((row['engine'],row['samples']),[]).append(row)
result=[]
for (engine,samples),rows in groups.items():
    warm=[row['total_s'] for row in rows if not row['cold']]
    cold=[row['total_s'] for row in rows if row['cold']]
    result.append(dict(engine=engine,samples=samples,measured_frames=len(rows),warm_frames=len(warm),
        first_frame_s=cold[0] if cold else None,warm_mean_s=statistics.mean(warm),
        warm_min_s=min(warm),warm_max_s=max(warm),projected_movie_frames=shot['frames'],
        projected_movie_s=shot['frames']*statistics.mean(warm),
        projection=True,excludes=['simulation bake','scene construction','process startup','encoding']))
print(json.dumps(result,indent=2))
