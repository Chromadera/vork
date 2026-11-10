#!/usr/bin/env python3
"""Render resolver/cache regressions. Run with a real GPU; uses only the Python standard library.

python dependency-render-matrix.py --binary /path/to/vork --out /tmp/matrix
Copy the compute-trace fixture beside this script when running in a GPU packet.
The companion Rust dependency_layout_matrix covers filesystem layouts on both
Slang targets without GPU hardware. This harness exercises the renderer cache.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import struct
import zlib

def center_pixel(path):
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, compressed = 8, bytearray()
    while pos < len(data):
        length = struct.unpack_from(">I", data, pos)[0]
        kind, payload = data[pos+4:pos+8], data[pos+8:pos+8+length]
        if kind == b"IHDR":
            width,height,depth,color,_,_,interlace = struct.unpack(">IIBBBBB",payload)
        elif kind == b"IDAT": compressed.extend(payload)
        pos += length+12
    assert depth in (8,16) and color in (2,6) and interlace == 0
    channels = 3 if color == 2 else 4
    bpp = channels*(depth//8)
    stride = width*bpp
    raw = zlib.decompress(compressed)
    previous = bytearray(stride)
    for y in range(height//2+1):
        offset = y*(stride+1)
        filt = raw[offset]
        row = bytearray(raw[offset+1:offset+1+stride])
        for x in range(stride):
            left = row[x-bpp] if x >= bpp else 0
            up = previous[x]
            corner = previous[x-bpp] if x >= bpp else 0
            estimate = left+up-corner
            distances = [abs(estimate-left),abs(estimate-up),abs(estimate-corner)]
            paeth = [left,up,corner][distances.index(min(distances))]
            row[x] = (row[x] + [0,left,up,(left+up)//2,paeth][filt]) % 256
        previous = row
    start = width//2*bpp
    return [row[start+i*(depth//8)] for i in range(3)]

p = argparse.ArgumentParser()
p.add_argument('--binary', default='vork')
p.add_argument('--out', type=Path, required=True)
p.add_argument('--case', choices=['surface', 'compute', 'compute-trace'])
a = p.parse_args()
binary = shutil.which(a.binary) or str(Path(a.binary).resolve())
a.out.mkdir(parents=True, exist_ok=True)
package = Path(binary).parent
libdirs = [str(package / d) for d in ('slang/lib', 'oidn/lib') if (package / d).is_dir()]
if libdirs:
    os.environ['LD_LIBRARY_PATH'] = ':'.join(libdirs + [os.environ.get('LD_LIBRARY_PATH', '')])
fixture = Path(__file__).with_name('compute-trace.json')
if not fixture.exists():
    fixture = Path(__file__).resolve().parents[2] / 'crates/vork-render/tests/fixtures/compute-trace.json'
base = json.loads(fixture.read_text())
results = []
for case in ([a.case] if a.case else ['surface', 'compute', 'compute-trace']):
    root = (a.out / case).resolve()
    root.mkdir(parents=True, exist_ok=True)
    cache = root / 'cache'
    for name, color in [('A', '1.,0.,0.'), ('B', '0.,1.,0.')]:
        shot = root / name
        shot.mkdir(exist_ok=True)
        scene = copy.deepcopy(base)
        scene['render']['resolution'] = [64, 64]
        if case == 'surface':
            del scene['programs']['bake']
            scene['programs']['show'] = {'file': 'program.slang'}
            program = 'import vork; import palette; struct Inputs {}; Closure surface(ShadingContext ctx, Inputs i){return unlit(palette_colour());}'
        else:
            del scene['programs']['bake']['source']
            scene['programs']['bake']['file'] = 'program.slang'
            expression = 'palette_colour()'
            if case == 'compute-trace':
                expression = 'trace(float3(2.,2.,0.),float3(0.,-2.,0.),0.,3.,255u,1u).hit ? palette_colour() : float3(0.,0.,1.)'
            program = 'import vork; import palette; struct Inputs {}; void compute(ComputeContext ctx, Inputs i){color_store(ctx.id.x, ' + expression + ');}'
        (shot / 'scene.json').write_text(json.dumps(scene))
        (shot / 'program.slang').write_text(program)
        (shot / 'palette.slang').write_text('public float3 palette_colour(){return float3(' + color + ');}')
    for step, folder, absolute, expected in [('A-cold','A',False,'red'), ('A-warm','A',False,'red'), ('B-cold','B',False,'green'), ('B-warm','B',False,'green'), ('B-absolute','B',True,'green'), ('B-edit','B',False,'yellow'), ('B-edit-warm','B',False,'yellow')]:
        shot = root / folder
        if step == 'B-edit':
            (shot/'palette.slang').write_text('public float3 palette_colour(){return float3(1.,1.,0.);}')
        frame = root / (step + '.png')
        cmd = [binary,'render',str(shot/'scene.json') if absolute else 'scene.json','--out',str(frame)]
        run = subprocess.run(cmd, cwd=shot, env=dict(os.environ,VORK_CACHE_DIR=str(cache)), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (root/(step+'.log')).write_text(run.stdout)
        pixel = center_pixel(frame) if run.returncode == 0 else None
        right = pixel is not None and {'red':pixel[0]>pixel[1]+50, 'green':pixel[1]>pixel[0]+50, 'yellow':min(pixel[:2])>pixel[2]+50}[expected]
        warm = 'shader jobs 0/0' in run.stdout
        ok = right and (warm if 'warm' in step or absolute else True)
        row = dict(case=case,step=step,expected=expected,pixel=pixel,warm=warm,ok=ok,exit=run.returncode)
        results.append(row)
        print(json.dumps(row),flush=True)
(a.out/'results.json').write_text(json.dumps(results,indent=2))
raise SystemExit(0 if all(r['ok'] for r in results) else 1)
