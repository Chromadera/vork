#!/usr/bin/env python3
"""Prepare a small closed-shutter diagnostic; preserve the liquid's physical setup."""
import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('out', type=Path, help='new scene JSON path outside the checkout')
a = p.parse_args()
root = Path(__file__).resolve().parent
out = a.out.resolve()
if out.exists() or out.is_relative_to(root.parents[1]):
    p.error('out must be a new path outside the checkout')
s = json.loads((root/'scene.json').read_text())

def resolve(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == 'file' and isinstance(child, str):
                value[key] = str((root/child).resolve())
            else:
                resolve(child)
    elif isinstance(value, list):
        for child in value:
            resolve(child)

resolve(s)
s['render']['resolution'] = [160, 90]
s['render']['motion_blur']['shutter'] = 0
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(s, indent=2)+'\n')
print(out)
