"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
from pathlib import Path
import numpy as np,pycolmap,cv2
from scipy.spatial.transform import Rotation
from scipy.optimize import least_squares
import os
p=Path(os.environ['VFX_NEON_EVIDENCE']);d=dict(np.load(p/'tracked-ecc.npz'));out=p/'ecc-model';out.mkdir(exist_ok=True);ids=d['train'];K=d['K'];F=len(d['xy']);N=len(ids);poses=np.array(d['poses'])
(out/'cameras.txt').write_text(f'1 SIMPLE_RADIAL 1920 825 {K[0,0]} {K[0,2]} {K[1,2]} {d["dist"][0]}\n')
with (out/'images.txt').open('w') as f:
 for n,pose in enumerate(poses):
  q=Rotation.from_rotvec(pose[:3]).as_quat(scalar_first=True);f.write(f'{n+1} '+' '.join(map(str,np.r_[q,pose[3:]]))+f' 1 {n:04d}.png\n')
  f.write(' '.join(f'{d["xy"][n,i,0]} {d["xy"][n,i,1]} {j+1}' for j,i in enumerate(ids))+'\n')
with (out/'points3D.txt').open('w') as f:
 for j,i in enumerate(ids):f.write(f'{j+1} '+' '.join(map(str,d['xyz'][i]))+' 255 255 255 0 '+' '.join(f'{n+1} {j}' for n in range(F))+'\n')
r=pycolmap.Reconstruction();r.read_text(out);o=pycolmap.BundleAdjustmentOptions();o.refine_focal_length=True;o.refine_extra_params=False;o.refine_principal_point=False
pycolmap.bundle_adjustment(r,o);r.write(out);print(r.summary(),flush=True)
K[0,0]=K[1,1]=r.camera(1).params[0];print('focal',K[0,0],flush=True)
xyz=d['xyz'].copy()
for j,i in enumerate(ids):xyz[i]=r.point3D(j+1).xyz
for n in range(F):
 pose=r.image(n+1).cam_from_world();poses[n]=np.r_[pose.rotation.matrix().shape and cv2.Rodrigues(pose.rotation.matrix())[0].ravel(),pose.translation]
def project(poses,xyz):
 rot=Rotation.from_rotvec(poses[:,:3]).as_matrix();q=np.einsum('fij,nj->fni',rot,xyz)+poses[:,None,3:];q=q[...,:2]/q[...,2:];q*=1+d['dist'][0]*(q*q).sum(-1,keepdims=True);return q*K[0,0]+K[:2,2]
for i in d['test']:
 def fun(pt):return (project(poses[[0,-1]],pt[None])[:,0]-d['xy'][[0,-1],i]).ravel()
 xyz[i]=least_squares(fun,xyz[i],max_nfev=100).x
err=np.linalg.norm(project(poses,xyz[d['test']])-d['xy'][:,d['test']],axis=-1)
d.update(poses=poses,xyz=xyz,errors=err);np.savez_compressed(p/'ecc-adjusted.npz',**d);print('heldout',err.mean(),err.max(),np.percentile(err,[50,90,99]),flush=True)
