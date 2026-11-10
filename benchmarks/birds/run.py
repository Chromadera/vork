#!/usr/bin/env python3
"""Rerun the million-bird shot and Blender's draw-only comparison on a real GPU."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import time

SHOTS = {
    'birds2': dict(frames=192, start_frame=144, ticks=[384,448,512,576,640,704,768,832]),
    'skim': dict(frames=192, start_frame=216, ticks=[576,640,704,768,832,896,960,1024]),
    'dive': dict(frames=240, start_frame=216, ticks=[576,640,704,768,896,960,1024,1088]),
    'reveal': dict(frames=288, start_frame=288, ticks=[768,832,960,1024,1152,1216,1344,1408]),
}
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--shot', choices=SHOTS, default='skim')
p.add_argument('--vork', default='vork')
p.add_argument('--blender', default='blender')
p.add_argument('--repetitions', type=int, default=5)
p.add_argument('--skip-blender', action='store_true')
p.add_argument('--prepare-only', action='store_true', help='write the renderable shot under OUT/wetland and stop; no renderer needed')
a = p.parse_args()
if a.repetitions < 1: p.error('--repetitions must be positive')
root = a.out.resolve(); root.mkdir(parents=True, exist_ok=False)
here = Path(__file__).resolve().parent
repo = here.parents[1]
binary = str(Path(shutil.which(a.vork) or a.vork).resolve())
blender = shutil.which(a.blender) or a.blender
shot = dict(SHOTS[a.shot], name=a.shot, scene='wetland/scene.json')
env = dict(os.environ, VORK_CACHE_DIR=str(root/'cache'), VORK_BIRD_BENCH_ROOT=str(root))
rows = []
def run(label, command, phase):
    start = time.perf_counter()
    with (root/(label+'.log')).open('w') as log:
        result = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
    row = dict(label=label, phase=phase, command=command, seconds=time.perf_counter()-start, returncode=result.returncode)
    rows.append(row)
    (root/'timings.json').write_text(json.dumps(rows, indent=2)+'\n')
    print(json.dumps(row), flush=True)
    if result.returncode: raise SystemExit(result.returncode)

run('assets', [sys.executable, str(repo/'examples/murmuration/prepare_launch.py'), str(root/'wetland')], 'preparation')
scene = json.loads((here/(a.shot+'.json.in')).read_text())
series = 'birds2' if a.shot == 'birds2' else 'birds3'
for source in (here/'shaders'/series).glob('*.slang'):
    shutil.copy2(source, root/'wetland'/source.name)
# Initial writable state is zeroed, quaternions start at identity, and the
# million wing phases retain the authored deterministic sequence.
for name, buf in scene['buffers'].items():
    file = buf.get('file', '')
    if not file or file.startswith('assets/'): continue
    if name == 'phases': data = b''.join(struct.pack('<f', (i % 24)*.137) for i in range(buf['count']))
    elif buf['type'] == 'vec4': data = struct.pack('<4f', 0,0,0,1)*buf['count']
    else:
        stride = 16 if file == 'state.bin' else {'vec3':12,'u32':4,'f32':4}[buf['type']]
        data = bytes(stride*buf['count'])
    (root/'wetland'/file).write_bytes(data)
(root/'wetland/scene.json').write_text(json.dumps(scene)+'\n')
(root/'shot.json').write_text(json.dumps(shot,indent=2)+'\n')
if a.prepare_only: raise SystemExit(0)
(root/'version.txt').write_text(subprocess.check_output([binary,'--version'],text=True))
(root/'scene-sha256.txt').write_text(hashlib.sha256((root/'wetland/scene.json').read_bytes()).hexdigest()+'\n')
run('setup', [binary,'setup'], 'setup')
(root/'results').mkdir()
for profile in ['previs','final']:
    base = [binary,'render',str(root/'wetland/scene.json'),'--profile','std/'+profile,'--diagnostics']
    run(profile+'-warmup', base+['--at',str(shot['start_frame']/24),'--out',str(root/(profile+'-warmup.png'))], 'warmup')
    for rep in range(a.repetitions):
        run(profile+'-'+str(rep),base+['--frame-start',str(shot['start_frame']),'--frame-end',str(shot['start_frame']+shot['frames']-1),'--out',str(root/(profile+'-'+str(rep)+'.mp4'))], 'warm-movie')
if not a.skip_blender:
    import numpy as np
    import blake3
    # Keep baking separate from the movie cache: selecting its newest checkpoint
    # must never pick a later movie frame when seeking to an earlier bake tick.
    bake_cache = root/'bake-cache'
    env['VORK_CACHE_DIR'] = str(bake_cache)
    layout = {}; offset = 0
    for name in sorted(scene['simulation']['state']):
        b = scene['buffers'][name]; width = {'vec3':3,'vec4':4,'u32':1}[b['type']]
        count = b.get('count',len(b.get('data',[0])))
        layout[name] = dict(offset=offset,count=count,width=width,type=b['type']); offset += width*count*4
    (root/'checkpoint-layout.json').write_text(json.dumps(layout,indent=2)+'\n')
    for tick in shot['ticks']:
        run('bake-'+str(tick),[binary,'render',str(root/'wetland/scene.json'),'--profile','std/previs','--at',str(tick/60),'--set','resolution=320x180','--out',str(root/('bake-'+str(tick)+'.png'))], 'simulation-bake-excluded')
        cache = max(bake_cache.glob('*.simulation-v1.entry'),key=lambda path:path.stat().st_mtime)
        data = cache.read_bytes()
        if blake3.blake3(data[32:]).digest()!=data[:32] or len(data)!=offset+32:
            raise RuntimeError('Simulation checkpoint format changed; update the benchmark reader before comparing.')
        arrays = {name:np.frombuffer(data[32:],dtype='<u4' if b['type']=='u32' else '<f4',count=b['count']*b['width'],offset=b['offset']).reshape(b['count'],b['width']).copy()
                  for name,b in layout.items() if name in ['translation','rotation','v','far_scale','near_count','near_translation','near_rotation','near_scale']}
        np.savez(root/('state-'+str(tick)+'.npz'),**arrays)
    adapter = repo/'integrations/blender'/(series+'_benchmark.py')
    (root/'blender-version.txt').write_text(subprocess.check_output([blender,'--version'],text=True))
    for mode in ['setup','eevee','eevee-previs','cycles']:
        run('blender-'+mode,[blender,'-b','--disable-autoexec','--python',str(adapter),'--',mode], 'blender-'+mode)
