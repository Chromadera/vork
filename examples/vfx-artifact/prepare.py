#!/usr/bin/env python3
"""Prepare the licensed plate outside the checkout; invoke open plate tools.

Python dependencies: NumPy, Pillow, SciPy, OpenCV. Model tools separately need
torch, upstream sam2, and transformers. All are external authoring tools.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from track import solve
from build import build

HERE=Path(__file__).resolve().parent


def reflections(media):
    """Low-frequency room estimate from the plate, not a measured HDR panorama."""
    import cv2
    import numpy as np
    cap=cv2.VideoCapture(str(media/'plate.mp4'))
    ok,frame=cap.read();cap.release()
    if not ok: raise RuntimeError('cannot read reflection reference')
    frame=cv2.GaussianBlur(frame,(0,0),5.)
    source=frame.astype(np.float32)/255
    source=np.where(source<=.04045,source/12.92,((source+.055)/1.055)**2.4)
    # Repeat the visible room behind camera; the explicit practical/fill lights
    # supply the key direction and clipped-source energy absent in this plate.
    yy,xx=np.mgrid[0:512,0:1024]
    sx=((xx/1024*2+.15)%1*1919).astype('float32')
    sy=(yy/511*1079).astype('float32')
    panorama=cv2.remap(source,sx,sy,cv2.INTER_LINEAR)
    cv2.imwrite(str(media/'room.hdr'),panorama)


def calibrate_depth(media):
    import cv2
    import numpy as np
    paths=sorted((media/'depth').glob('*.npy'))
    if len(paths)!=200: raise RuntimeError('run depth.py first: expected 200 depth maps')
    out=media/'calibrated-depth';out.mkdir(exist_ok=True)
    # Relative depth is underdetermined. Set a local relief scale, anchored to
    # the visible front table and the face; do not interpret this as a scan.
    anchor=float(np.median(np.load(paths[0])[470:480,450:470]))
    for index,path in enumerate(paths):
        d=np.load(path)
        z=np.clip(1.55+.055*(anchor-d),.6,4.)
        z=cv2.GaussianBlur(z,(0,0),1.6)
        cv2.imwrite(str(out/f'{index:06d}.png'),np.uint16(z/8*65535))
    subprocess.run(['ffmpeg','-v','error','-y','-framerate','25','-i',str(out/'%06d.png'),
                    '-c:v','ffv1','-pix_fmt','gray16le',str(media/'relative-depth.mkv')],check=True)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets-dir',type=Path,required=True)
    p.add_argument('--checkpoint',type=Path)
    p.add_argument('--models',action='store_true',help='run SAM 2 and Depth Anything locally on CUDA')
    p.add_argument('--finish',action='store_true',help='use completed external model results')
    a=p.parse_args();media=a.assets_dir.resolve()
    if HERE.parents[1] in media.parents or media==HERE.parents[1]: p.error('media must be outside checkout')
    media.mkdir(parents=True,exist_ok=True)
    asset=json.loads((HERE/'assets.json').read_text())[0]
    source=media/'source7983323.mp4'
    if not source.exists():
        subprocess.run(['curl','-L','--fail',asset['url'],'-o',str(source)],check=True)
    digest=hashlib.sha256(source.read_bytes()).hexdigest()
    if digest!=asset['sha256']: raise RuntimeError(f'unexpected source SHA256 {digest}')
    if not (media/'plate.mp4').exists():
        subprocess.run(['ffmpeg','-v','error','-y','-i',str(source),'-vf',
            'scale=2048:1080,crop=1920:1080','-t','8','-c:v','libx264','-crf','16','-an',str(media/'plate.mp4')],check=True)
    solve(media)
    reflections(media)
    if a.models:
        if not a.checkpoint: p.error('--models needs --checkpoint')
        from mattes import run as matte
        from depth import run as depth
        matte(media,a.checkpoint);matte(media,a.checkpoint,hand=True);depth(media)
    if a.models or a.finish:
        if not (media/'person.mkv').exists(): raise RuntimeError('SAM person.mkv missing')
        calibrate_depth(media)
        build(media)
    (media/'provenance.json').write_text(json.dumps(dict(asset=asset,
        source_sha256=digest,crop='2048x1080 then centre 1920x1080',seconds=[0,8],fps=25,
        camera='Full-length LK, reference patch refinement, rigid local camera pose; held-out validation',
        matte='SAM 2.1 Hiera Small',depth='Depth Anything V2 Small; locally calibrated relief'),indent=2)+'\n')


if __name__=='__main__': main()
