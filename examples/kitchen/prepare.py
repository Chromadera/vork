#!/usr/bin/env python3
"""Export a user-supplied Cozy Kitchen .blend and add a gentle camera orbit."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('source', type=Path, help='Blender 3.5 splash .blend; see README and credits')
p.add_argument('out', type=Path, help='new directory outside the checkout')
p.add_argument('--blender', default='blender')
a = p.parse_args()
root = Path(__file__).resolve().parents[2]
a.out = a.out.resolve()
if a.out.exists() or a.out.is_relative_to(root):
    p.error('out must be a new directory outside the checkout')
if hashlib.sha256(a.source.read_bytes()).hexdigest() != '5f27c41cf828ca15ffd06fc3ad3d971343f6af4a29828c834252229a26dce395':
    p.error('source hash differs from the pinned Blender 3.5 Cozy Kitchen download')
subprocess.run([a.blender, '-b', '--disable-autoexec', str(a.source.resolve()),
                '--python-exit-code', '1', '--python', str(root/'integrations/blender/export.py'),
                '--', '--out', str(a.out)], check=True)
s = json.loads((a.out/'shot.json').read_text())
# Use the launch adaptation's compact shared material program. The native
# per-material translation is retained in export-report.json for inspection.
look_dir = Path(__file__).with_name('look')
look = json.loads((look_dir/'materials.json.in').read_text())
asset = next(n for n in s['nodes'] if n['type'] == 'asset')
if asset.get('materials') != look['material_map']:
    raise ValueError('Material layout differs from the Blender 3.5 Cozy Kitchen source')
s['materials'].update(look['materials'])
s['programs'] = look['programs']
s['images'].update(look['images'])
for file in [v['file'] for v in look['images'].values()] + ['kitchen_surface.slang']:
    shutil.copyfile(look_dir/file, a.out/file)
s['render'].update(resolution=[640,360], fps=24, duration=7)
tracks=[]
for node in s['nodes']:
    if node['type'] != 'camera':
        continue
    position=node.get('translation',[0,0,0]); pivot=[0,1.9,0]
    rotation=node.get('rotation',[0,0,0,1]);x,y,z,w=rotation
    positions=[];rotations=[]
    for frame in range(169):
        t=frame/24;u=t/7;angle=math.radians(24)*(u*u*(3-2*u)-.5)
        c,si=math.cos(angle),math.sin(angle)
        px,py,pz=[position[i]-pivot[i] for i in range(3)]
        positions.append([t,[pivot[0]+c*px+si*pz,pivot[1]+py,pivot[2]-si*px+c*pz]])
        q,r=math.sin(angle/2),math.cos(angle/2)
        rotations.append([t,[r*x+q*z,r*y+q*w,r*z-q*x,r*w-q*y]])
    tracks.extend([dict(path=['nodes',node['name'],'translation'],keys=positions),dict(path=['nodes',node['name'],'rotation'],keys=rotations)])
s.setdefault('animation',[]).append(dict(tracks=tracks))
(a.out/'shot.json').write_text(json.dumps(s,indent=2)+'\n')
shutil.copyfile(Path(__file__).with_name('CREDITS.md'),a.out/'CREDITS.md')
print(a.out/'shot.json')
