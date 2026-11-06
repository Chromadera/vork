#!/usr/bin/env python3
"""Solve static wall texture and export a camera with independent wall checks.

Planar self-calibration has scale/focal ambiguities. This is local wall
registration, not a surveyed ground solve or a pedestrian-occlusion certificate.
"""
import argparse,json,os,subprocess,sys
from pathlib import Path
import cv2,numpy as np
from scipy.spatial.transform import Rotation

def export(media,evidence):
    p=evidence;m=media
    D=np.load(p/'planar-adjusted.npz');K=D['K'];poses=D['poses'];R0=cv2.Rodrigues(poses[0,:3])[0];t0=poses[0,3:];q=D['xyz'][D['train']]@R0.T+t0
    _,_,V=np.linalg.svd(q-q.mean(0));normal=V[-1];normal*=(-1 if normal[0]>0 else 1);offset=normal @ q.mean(0)
    def wall(uv):
     ray=np.linalg.inv(K)@np.r_[uv,1];return ray*(offset/(normal@ray))
    up=wall([1140,310])-wall([1140,460]);up/=np.linalg.norm(up);right=-normal;back=np.cross(right,up);back/=np.linalg.norm(back);up=np.cross(back,right);B=np.array([right,up,back]);foot=up*(up@wall([1140,490]));scale=.5
    world=lambda v:((np.array(v)-foot)@B.T*scale)
    worldposes=[]
    for n,pose in enumerate(poses):
     R=cv2.Rodrigues(pose[:3])[0];C=-R.T@pose[3:];Cfirst=R0@C+t0;Q=B@R0@R.T @ np.diag([1,-1,-1]);worldposes.append(dict(time=n/24,position=world(Cfirst).tolist(),rotation=Rotation.from_matrix(Q).as_quat().tolist()))
    cam=dict(wall_x=float(np.mean(world(q)[:,0])),base=[-1.2,1.8,-14],intrinsics={'f':float(K[0,0])},poses=worldposes)
    (media/'camera.json').write_text(json.dumps(cam,indent=2)+'\n')
    ids=D['test'];xy=D['xy']
    wallx=float(np.mean(world(q)[:,0]))
    contacts=np.array([[wallx,2.12,-16],[wallx,2.12,-16.2],[wallx,2.12,-15.8],[wallx,2.312,-16.8]])
    xyz=((contacts/scale)@B+foot-t0)@R0
    pred=[];track=[];features=[]
    for n,pose in enumerate(poses):
     R=cv2.Rodrigues(pose[:3])[0];v=xyz@R.T+pose[3:];pred.append(v[:,:2]/v[:,2:]*K[0,0]+K[:2,2])
     H,_=cv2.findHomography(xy[0,ids].astype(float),xy[n,ids].astype(float),0)
     a=np.c_[pred[0],np.ones(len(contacts))]@H.T;track.append(a[:,:2]/a[:,2:])
     z=D['xyz'][ids]@R.T+pose[3:];features.append(z[:,:2]/z[:,2:]*K[0,0]+K[:2,2])
    pred=np.array(pred);track=np.array(track);err=np.linalg.norm(pred-track,axis=-1)
    report=dict(status='PROVISIONAL; wall homography comparison, not direct tracked contact texture',
     caveat='Reference motion comes from held-out wall texture. Contacts are virtual points interpolated/extrapolated by a per-frame homography; these are not independent physical survey points. Ground contact and pedestrian occlusion are not certified.',
     coordinate_convention='OpenCV pixel-center coordinates, native 1920x825 plate; rendered camera shift [-0.5/1920,0] reconciles pixel centers with K; add 127 to Y for padded delivery',frames=len(poses),fps=24,contact_world=contacts.tolist(),contact_mean_px=float(err.mean()),contact_max_px=float(err.max()),
     contact_projected_px=pred.tolist(),contact_reference_px=track.tolist(),contact_error_px=err.tolist(),
     heldout_feature_ids=ids.tolist(),heldout_tracked_px=xy[:,ids].tolist(),heldout_projected_px=np.array(features).tolist(),
     heldout_mean_px=float(D['errors'].mean()),heldout_max_px=float(D['errors'].max()))
    (evidence/'contact-validation.json').write_text(json.dumps(report,indent=2)+'\n')
    print('wall contact homography comparison',err.mean(),err.max(),np.max(err,axis=0))

def solve(media,evidence,export_only=False):
    checkout=Path(__file__).resolve().parents[2]
    for path in [media,evidence]:
        if path==checkout or checkout in path.parents:raise ValueError('Keep media and evidence outside the checkout')
    evidence.mkdir(parents=True,exist_ok=True)
    if not export_only:
        env=dict(os.environ,VFX_NEON_MEDIA=str(media),VFX_NEON_EVIDENCE=str(evidence))
        for name in ['solve','refit','track_dense','ecc','ecc_bundle','planar_bundle']:
            subprocess.run([sys.executable,str(Path(__file__).with_name('matchmove')/(name+'.py'))],env=env,check=True)
    export(media,evidence)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--media',type=Path,required=True);p.add_argument('--evidence',type=Path,required=True)
    p.add_argument('--export-only',action='store_true',help='Reuse an existing planar-adjusted.npz solve')
    a=p.parse_args();solve(a.media.resolve(),a.evidence.resolve(),a.export_only)
