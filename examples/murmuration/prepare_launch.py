#!/usr/bin/env python3
"""Prepare the million-bird skim shot outside the checkout (about 100 MB)."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import urllib.request

ROOT = Path(__file__).resolve().parent
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('out', type=Path)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
s = json.loads((ROOT / 'launch/skim.json.in').read_text())
for src in (ROOT / 'launch').glob('*.slang'):
    shutil.copyfile(src, a.out / src.name)
shutil.copytree(ROOT / 'assets/birds', a.out / 'assets/birds', dirs_exist_ok=True)
shutil.copyfile(ROOT / 'launch/dusk-weight.exr', a.out / 'assets/dusk-weight.exr')
shutil.copyfile(ROOT / 'CREDITS', a.out / 'CREDITS')
# Only the sky and grass textures are used by this shot.
for item in json.loads((ROOT / 'assets.lock.json').read_text()):
    if item['path'] != 'sky.hdr' and not item['path'].startswith('grass_bermuda_01/textures/'):
        continue
    target = a.out / 'assets' / item['path']
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == item['sha256']:
        continue
    request = urllib.request.Request(item['url'], headers={'User-Agent': 'vork-example/1.0'})
    data = urllib.request.urlopen(request, timeout=120).read()
    if hashlib.sha256(data).hexdigest() != item['sha256']:
        raise RuntimeError('Asset hash mismatch: ' + item['path'])
    target.write_bytes(data)
for name, b in s['buffers'].items():
    file = b.get('file', '')
    if not file or file.startswith('assets/'):
        continue
    if name == 'phases':
        data = b''.join(struct.pack('<f', (i % 24) * .137) for i in range(b['count']))
    elif b['type'] == 'vec4':
        data = struct.pack('<4f', 0, 0, 0, 1) * b['count']
    else:
        stride = 16 if file == 'state.bin' else {'vec3': 12, 'u32': 4}[b['type']]
        data = bytes(stride * b['count'])
    (a.out / file).write_bytes(data)
(a.out / 'scene.json').write_text(json.dumps(s) + '\n')
print(a.out / 'scene.json')
