"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
import cv2,numpy as np
from pathlib import Path
import os
p=Path(os.environ['VFX_NEON_EVIDENCE']);d=dict(np.load(p/'tracked-fixed.npz'));xy=d['xy'];ids=np.sort(np.r_[d['train'],d['test']]);F=len(xy)
c=cv2.VideoCapture(str(Path(os.environ['VFX_NEON_MEDIA'])/'track.mp4'));frames=[]
while True:
 ok,b=c.read()
 if not ok:break
 frames.append(cv2.cvtColor(b,cv2.COLOR_BGR2GRAY))
new=xy.copy();scores=np.ones((F,len(xy[0])));alive=np.ones(len(xy[0]),bool);radius=20;yy,xx=np.mgrid[-radius:radius+1,-radius:radius+1].astype(np.float32)
for n in range(1,F):
 for side in [False,True]:
  subset=ids[(xy[0,ids,0]>1000)==side];H,_=cv2.findHomography(xy[0,subset],xy[n,subset],cv2.RANSAC,2)
  for i in subset:
   x,y=xy[0,i];grid=np.stack([xx+x,yy+y,np.ones_like(xx)],-1)@H.T;grid=(grid[...,:2]/grid[...,2:]).astype(np.float32)
   center=grid[radius,radius].copy();grid+=xy[n,i]-center
   im=cv2.remap(frames[n],grid[:,:,0],grid[:,:,1],cv2.INTER_CUBIC)
   template=cv2.getRectSubPix(frames[0],(2*radius+1,)*2,(float(x),float(y)))
   try:score,W=cv2.findTransformECC(template,im,np.eye(2,3,dtype=np.float32),cv2.MOTION_TRANSLATION,(3,60,1e-6),None,3)
   except cv2.error:alive[i]=False;continue
   offset=W[:,2];J=np.stack([(grid[radius,radius+1]-grid[radius,radius-1])*.5,(grid[radius+1,radius]-grid[radius-1,radius])*.5],1)
   new[n,i]=xy[n,i]+J@offset;scores[n,i]=score
   alive[i] &= score>.85 and np.linalg.norm(offset)<5
 if n%20==0:print(n,'alive',sum(alive[ids]),flush=True)
ids=ids[alive[ids]];d.update(xy=new,train=ids[::2],test=ids[1::2]);np.savez_compressed(p/'tracked-ecc.npz',**d);np.savez_compressed(p/'ecc-scores.npz',scores=scores,alive=alive)
print('retained',len(ids),flush=True)
