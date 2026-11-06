#!/usr/bin/env python3
"""Bake mathematical samples from Blender 5.2.1's BSD-licensed OCIO config.
Run: uv run --with opencolorio==2.6.0 --with numpy --with OpenEXR==3.4.0 scripts/color/bake-agx-looks.py
No Blender renderer source is imported. None retains the original core LUT samples.
Writes the std/agx library display image; each band uses Filmic's layout.
"""
import hashlib
import json
from pathlib import Path
import tempfile
import urllib.request
import numpy as np
import PyOpenColorIO as ocio
import OpenEXR

ROOT = Path(__file__).resolve().parents[2]
BASE = 'https://raw.githubusercontent.com/blender/blender/v5.2.1/release/datafiles/colormanagement/'
LOOKS = ['Punchy', 'Greyscale', 'Very High Contrast', 'High Contrast',
         'Medium High Contrast', 'Base Contrast', 'Medium Low Contrast',
         'Low Contrast', 'Very Low Contrast']
with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)
    sources = {}
    for name in ['config.ocio', 'luts/AgX_Base_sRGB.cube', 'luts/luminance_compensation_bt2020.cube']:
        data = urllib.request.urlopen(BASE + name).read()
        path = tmp / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        sources[name] = hashlib.sha256(data).hexdigest()
    config = ocio.Config.CreateFromFile(str(tmp / 'config.ocio'))
    axis = np.r_[0., np.exp2(np.linspace(-12.47393, 12.5260688117, 65)[1:])]
    b, g, r = np.meshgrid(axis, axis, axis, indexing='ij')
    grid = np.stack([r,g,b], axis=-1).astype(np.float32).reshape(-1,3)
    display = ocio.DisplayViewTransform(src='Linear Rec.709', display='sRGB', view='AgX')
    display.setLooksBypass(True)
    # Retain the original Blender PNG16-derived None samples in the single library image.
    neutral = OpenEXR.File(str(ROOT / 'libraries/blender/agx.exr')).channels()['RGB'].pixels[:65].copy()
    bands = [neutral]
    for look in LOOKS:
        transform = ocio.GroupTransform()
        transform.appendTransform(ocio.LookTransform(src='Linear Rec.709', dst='Linear Rec.709', looks='AgX - '+look))
        transform.appendTransform(display)
        rgb = grid.copy()
        config.getProcessor(transform).getDefaultCPUProcessor().applyRGB(rgb)
        bands.append(rgb.astype(np.float16).reshape(65,65,65,3).transpose(1,0,2,3).reshape(65,65*65,3))
    image = np.ascontiguousarray(np.concatenate(bands))
    OpenEXR.File({'compression': OpenEXR.ZIP_COMPRESSION, 'type': OpenEXR.scanlineimage}, {'RGB': image}).write(str(ROOT / 'libraries/blender/agx.exr'))
    (ROOT / 'libraries/blender/agx-manifest.json').write_text(json.dumps({
        'blender': '5.2.1', 'display': 'sRGB', 'view': 'AgX', 'looks': ['None', *LOOKS],
        'size': 65, 'min_log2': -12.47393, 'max_log2': 12.5260688117,
        'format': 'OpenEXR half RGB, ZIP; sample (r,g,b) of look L at x = 65 b + r, y = 65 L + g; None retains the original PNG16-derived samples',
        'source_sha256': sources, 'ocio': ocio.__version__}, indent=2)+'\n')
