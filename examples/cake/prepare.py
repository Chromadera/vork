#!/usr/bin/env python3
"""Download the pinned CC0 cake assets and prepare the shot outside the checkout."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import urllib.request

ROOT = Path(__file__).resolve().parent
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('out', type=Path)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
for item in json.loads((ROOT / 'assets.lock.json').read_text()):
    target = a.out / 'assets' / item['path']
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == item['sha256']:
        continue
    print('Fetching', item['path'], flush=True)
    request = urllib.request.Request(item['url'], headers={'User-Agent': 'vork-example/1.0'})
    data = urllib.request.urlopen(request, timeout=180).read()
    if hashlib.sha256(data).hexdigest() != item['sha256']:
        raise RuntimeError('Asset hash mismatch: ' + item['path'])
    target.write_bytes(data)
shutil.copyfile(ROOT / 'scene.json.in', a.out / 'scene.json')
shutil.copyfile(ROOT / 'CREDITS.md', a.out / 'CREDITS.md')
print(a.out / 'scene.json')
