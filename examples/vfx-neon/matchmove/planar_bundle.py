"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
from pathlib import Path
import numpy as np,cv2
from scipy.optimize import least_squares
from scipy.spatial.transform import Rotation
from scipy.sparse import lil_matrix
import os
p=Path(os.environ['VFX_NEON_EVIDENCE']);d=dict(np.load(p/'ecc-adjusted.npz'));ids=d['train'];F=len(d['poses']);N=len(ids);K=d['K'].copy()
points=d['xyz'][ids];origin=points.mean(0);_,_,V=np.linalg.svd(points-origin);basis=V.copy()
if np.linalg.det(basis)<0:basis[2]*=-1
uv=(points-origin)@basis[:2].T
poses=[]
for pose in d['poses']:
 R=cv2.Rodrigues(pose[:3])[0];poses.append(np.r_[cv2.Rodrigues(R@basis.T)[0].ravel(),R@origin+pose[3:]])
poses=np.array(poses)
def project(ps,xy):
 R=Rotation.from_rotvec(ps[:,:3]).as_matrix();q=np.einsum('fij,nj->fni',R,np.c_[xy,np.zeros(len(xy))])+ps[:,None,3:]
 return q[...,:2]/q[...,2:]*K[0,0]+K[:2,2]
def unpack(x):return np.vstack([poses[0],x[:6*(F-1)].reshape(-1,6)]),x[6*(F-1):].reshape(-1,2)
def fun(x):
 ps,pt=unpack(x);return (project(ps,pt)-d['xy'][:,ids]).ravel()
S=lil_matrix((F*N*2,(F-1)*6+N*2),dtype=int)
for n in range(F):
 for j in range(N):
  row=(n*N+j)*2
  if n:S[row:row+2,(n-1)*6:n*6]=1
  S[row:row+2,(F-1)*6+j*2:(F-1)*6+j*2+2]=1
x=np.r_[poses[1:].ravel(),uv.ravel()]
r=least_squares(fun,x,jac_sparsity=S,loss='soft_l1',f_scale=.5,max_nfev=200,ftol=1e-9,verbose=1)
ps,pt=unpack(r.x);xyz=d['xyz'].copy();xyz[ids]=pt@basis[:2]+origin
testuv=(xyz[d['test']]-origin)@basis[:2].T
for j,i in enumerate(d['test']):
 fit=least_squares(lambda q:(project(ps[[0,-1]],q[None])[:,0]-d['xy'][[0,-1],i]).ravel(),testuv[j]);testuv[j]=fit.x
xyz[d['test']]=testuv@basis[:2]+origin
out=[]
for pose in ps:
 R=cv2.Rodrigues(pose[:3])[0]@basis;out.append(np.r_[cv2.Rodrigues(R)[0].ravel(),pose[3:]-R@origin])
err=np.linalg.norm(project(ps,testuv)-d['xy'][:,d['test']],axis=-1)
d.update(xyz=xyz,poses=np.array(out),errors=err);np.savez_compressed(p/'planar-adjusted.npz',**d)
print('heldout',err.mean(),err.max(),np.percentile(err,[50,90,99]),flush=True)
