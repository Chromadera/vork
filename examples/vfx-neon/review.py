#!/usr/bin/env python3
"""Export every encoded frame, wall-lock overlays and quarter-speed review movies."""
import argparse,json,subprocess,hashlib
from pathlib import Path
import cv2,numpy as np

def review(media,movies,evidence):
    checkout=Path(__file__).resolve().parents[2]
    if evidence==checkout or checkout in evidence.parents:raise ValueError('Keep evidence outside the checkout')
    evidence.mkdir(parents=True,exist_ok=True)
    lock=json.loads((evidence/'contact-validation.json').read_text())
    cam=json.loads((media/'camera.json').read_text());scene=json.loads((media/'clean.json').read_text())
    tracks={tuple(t['path']):t for t in scene['animation'][0]['tracks']}
    for field,key in [('translation','position'),('rotation','rotation')]:
        assert tracks[('nodes','camera',field)]['keys']==[[p['time'],p[key]] for p in cam['poses']]
    assert scene['materials']['camera_optics']['inputs']['lens']==cam['intrinsics']['f']*36/1920
    assert scene['materials']['camera_optics']['inputs']['shift']==[-.5/1920,0]
    results={'occlusion':'No people in this plate; pedestrian lighting and occlusion are not demonstrated.'}
    for name in ['clean','shot']:
        folder=evidence/name;folder.mkdir(exist_ok=True);cap=cv2.VideoCapture(str(movies/(name+'.mp4')))
        assert (cap.get(3),cap.get(4))==(1920,1080) and abs(cap.get(5)-24)<.001
        tiles=[]
        for n in range(120):
            ok,bgr=cap.read()
            if not ok:raise RuntimeError(f'{name}: missing frame {n}')
            cv2.imwrite(str(folder/f'{n:04d}.png'),bgr)
            small=cv2.resize(bgr,(480,270));cv2.putText(small,f'{n:03d} / {n/24:.3f}s',(8,22),0,.6,(255,255,255),1);tiles.append(small)
            if len(tiles)==12:
                cv2.imwrite(str(evidence/f'{name}-{n-11:03d}.jpg'),np.vstack([np.hstack(tiles[k:k+4]) for k in range(0,12,4)]));tiles=[]
            if name!='clean':continue
            annotated=bgr.copy()
            for pk,tk,color in [('contact_projected_px','contact_reference_px',(255,255,0)),('heldout_projected_px','heldout_tracked_px',(0,255,0))]:
                for a,b in zip(lock[pk][n],lock[tk][n]):
                    a=tuple(np.round(np.array(a)+[0,127]).astype(int));b=tuple(np.round(np.array(b)+[0,127]).astype(int))
                    cv2.drawMarker(annotated,a,color,cv2.MARKER_CROSS,10,1);cv2.circle(annotated,b,3,(0,0,255),1)
            (evidence/'lock').mkdir(exist_ok=True);cv2.imwrite(str(evidence/'lock'/f'{n:04d}.png'),annotated)
        if cap.read()[0]:raise RuntimeError(f'{name}: extra frames')
        with (movies/(name+'.mp4')).open('rb') as f:digest=hashlib.file_digest(f,'sha256').hexdigest()
        results[name]=dict(frames=120,fps=24,resolution=[1920,1080],sha256=digest)
        subprocess.run(['ffmpeg','-v','error','-y','-i',str(movies/(name+'.mp4')),'-vf','setpts=4*PTS','-an','-c:v','libx264','-crf','16',str(evidence/(name+'-slow.mp4'))],check=True)
    (evidence/'review.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(results))

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for key in ['media','movies','evidence']:p.add_argument('--'+key,type=Path,required=True)
    a=p.parse_args();review(a.media.resolve(),a.movies.resolve(),a.evidence.resolve())
