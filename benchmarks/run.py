#!/usr/bin/env python3
"""Measure shipped profiles; keep commands, failures, images and timing rows outside Git."""
import argparse
import json
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--vork', default='vork')
p.add_argument('--out', type=Path, required=True)
p.add_argument('--repetitions', type=int, default=5)
p.add_argument('--resolution', default='1920x1080')
p.add_argument('--scenes', help='Comma-separated scene names; default: every B-*.json')
p.add_argument('--profiles', default='previs,final,path', help='Comma-separated shipped profile names')
p.add_argument('--video', action='store_true', help='Time complete 240-frame clips instead of frame-zero PNGs')
a = p.parse_args()
if a.out.exists():
    p.error('--out must be a new directory')
if a.repetitions < 1:
    p.error('--repetitions must be positive')
a.out.mkdir(parents=True)
root = Path(__file__).resolve().parent
(a.out / 'version.txt').write_text(subprocess.check_output([a.vork, '--version'], text=True))
rows = []

def run(name, command, phase):
    started = time.perf_counter()
    with (a.out / (name + '.log')).open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    row = dict(name=name, phase=phase, seconds=time.perf_counter()-started,
               returncode=result.returncode, command=command)
    rows.append(row)
    (a.out / 'timings.json').write_text(json.dumps(rows, indent=2) + '\n')
    print(json.dumps(row), flush=True)
    return result.returncode

if run('setup', [a.vork, 'setup'], 'setup'):
    raise SystemExit('Setup failed; see setup.log')
scenes = a.scenes.split(',') if a.scenes else sorted(path.stem for path in root.glob('B-*.json'))
for scene in scenes:
    for profile in a.profiles.split(','):
        base = [a.vork, 'render', str(root / (scene + '.json')),
                '--profile', 'std/' + profile, '--set', 'denoise=false',
                '--set', 'resolution=' + a.resolution, '--diagnostics']
        if not a.video:
            base += ['--at', '0']
        extension = '.mp4' if a.video else '.png'
        name = scene + '-' + profile
        if run(name+'-warmup', base + ['--out', str(a.out / (name+'-warmup'+extension))], 'warmup'):
            continue
        for i in range(a.repetitions):
            run(name+'-'+str(i), base + ['--out', str(a.out / (name+'-'+str(i)+extension))], 'warm')
if any(row['returncode'] for row in rows):
    raise SystemExit(1)
