#!/usr/bin/env python3
"""Bake Blender 5.2.1's Filmic view and its looks into libraries/blender/filmic.exr.
Run: uv run --with opencolorio==2.6.0 --with numpy --with OpenEXR==3.4.0 scripts/color/bake-filmic.py
Mathematical OCIO samples from Blender's BSD OCIO assets; no Blender renderer source
is imported. Source hashes are pinned in libraries/blender/filmic-manifest.json.

The image is the `lut` input of the blender/filmic display program: display-encoded
sRGB, half float RGB, lossless ZIP. Sample (r, g, b) of look L sits at pixel
x = 65 b + r, y = 65 L + g, with looks in manifest order. Each axis is exact zero then
2^(min_log2 + (max_log2 - min_log2) i / 64) for i = 1..64, the core display grid.
"""
from pathlib import Path
import argparse, hashlib, json, tempfile, urllib.request
import numpy as np
import OpenEXR
import PyOpenColorIO as ocio

parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument('--config', type=Path, help='Local Blender 5.2.1 config.ocio; otherwise download the pinned BSD OCIO assets')
args = parser.parse_args()
out = Path(__file__).resolve().parents[2] / 'libraries/blender'
manifest = json.loads((out / 'filmic-manifest.json').read_text())
size = manifest['size']
with tempfile.TemporaryDirectory() as cache:
    root = args.config.parent if args.config else Path(cache)
    for name, expected in manifest['source_sha256'].items():
        path = root / name
        if not args.config:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(urllib.request.urlopen('https://raw.githubusercontent.com/blender/blender/v5.2.1/release/datafiles/colormanagement/' + name).read())
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('Unexpected OCIO source hash: ' + name)
    config = ocio.Config.CreateFromFile(str(root / 'config.ocio'))
    axis = np.r_[0., np.exp2(np.linspace(manifest['min_log2'], manifest['max_log2'], size)[1:])]
    b, g, r = np.meshgrid(axis, axis, axis, indexing='ij')
    grid = np.stack([r, g, b], axis=-1).astype(np.float32).reshape(-1, 3)
    display = ocio.DisplayViewTransform(src='Linear Rec.709', display='sRGB', view='Filmic')
    display.setLooksBypass(True)
    bands = []
    for look in manifest['looks']:
        transform = ocio.GroupTransform()
        if look != 'None':
            transform.appendTransform(ocio.LookTransform(src='Linear Rec.709', dst='Linear Rec.709', looks=look))
        transform.appendTransform(display)
        rgb = grid.copy()
        config.getProcessor(transform).getDefaultCPUProcessor().applyRGB(rgb)
        # [b][g][r] -> rows g, columns 65 b + r.
        bands.append(rgb.astype(np.float16).reshape(size, size, size, 3).transpose(1, 0, 2, 3).reshape(size, size * size, 3))
image = np.ascontiguousarray(np.concatenate(bands))
path = out / 'filmic.exr'
OpenEXR.File({'compression': OpenEXR.ZIP_COMPRESSION, 'type': OpenEXR.scanlineimage}, {'RGB': image}).write(str(path))
print(path, image.shape, hashlib.sha256(path.read_bytes()).hexdigest())
