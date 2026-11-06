#!/usr/bin/env python3
"""Fetch hash-pinned ToyCar inputs for the turntable benchmark."""
import hashlib
import json
from pathlib import Path
import urllib.request

root = Path(__file__).resolve().parent
for row in json.loads((root / 'assets.lock.json').read_text())['files']:
    path = root / row['path']
    if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == row['sha256']:
        continue
    path.parent.mkdir(parents=True, exist_ok=True)
    data = urllib.request.urlopen(row['url'], timeout=120).read()
    if hashlib.sha256(data).hexdigest() != row['sha256']:
        raise RuntimeError('Asset checksum mismatch: ' + row['path'])
    path.write_bytes(data)
    print(row['path'])
