"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
from pathlib import Path
import cv2,numpy as np,pycolmap
import os
p=Path(os.environ['VFX_NEON_EVIDENCE']);r=pycolmap.Reconstruction(p/'fixed-sparse/0');cam=r.camera(1);f,cx,cy,k=cam.params;K=np.array([[f,0,cx],[0,f,cy],[0,0,1.]]);dist=np.zeros(5)
c=cv2.VideoCapture(str(Path(os.environ['VFX_NEON_MEDIA'])/'track.mp4'));frames=[]
while True:
 ok,bgr=c.read()
 if not ok:break
 frames.append(cv2.cvtColor(bgr,cv2.COLOR_BGR2GRAY))
mask=np.zeros(frames[0].shape,np.uint8)
cv2.fillPoly(mask,[np.array([[430,425],[750,395],[760,495],[450,690]])],255)
cv2.fillPoly(mask,[np.array([[1108,300],[1212,310],[1212,496],[1108,450]])],255)
a=cv2.goodFeaturesToTrack(frames[0],600,.008,7,mask=mask,blockSize=7);a=cv2.cornerSubPix(frames[0],a,(4,4),(-1,-1),(3,80,.0001));first=a.copy();alive=np.ones(len(a),bool);track=[a[:,0]];args=dict(winSize=(21,21),maxLevel=4,criteria=(3,100,.0001))
for n in range(1,len(frames)):
 b,st,err=cv2.calcOpticalFlowPyrLK(frames[n-1],frames[n],a,None,**args)
 back,bs,_=cv2.calcOpticalFlowPyrLK(frames[n],frames[n-1],b,None,**args)
 alive &= (st[:,0]>0)&(bs[:,0]>0)&(np.linalg.norm(back[:,0]-a[:,0],axis=1)<.35)&(err[:,0]<10)
 track.append(b[:,0]);a=b
track=np.array(track);print('alive',alive.sum(),flush=True);ids=np.where(alive)[0];train=ids[::2];test=ids[1::2]
ims=sorted(r.images.values(),key=lambda i:i.name);i0=ims[0];i1=ims[40]
P0=K @ i0.cam_from_world().matrix();P1=K @ i1.cam_from_world().matrix();h=cv2.triangulatePoints(P0,P1,track[0].T.astype(float),track[-1].T.astype(float));xyz=(h[:3]/h[3]).T
poses=[]
for n in range(len(frames)):
 ok,rv,tv,inliers=cv2.solvePnPRansac(xyz[train],track[n,train],K,dist,reprojectionError=3,iterationsCount=1000)
 if not ok:raise ValueError(n)
 rv,tv=cv2.solvePnPRefineLM(xyz[train[inliers[:,0]]],track[n,train[inliers[:,0]]],K,dist,rv,tv);poses.append(np.r_[rv.ravel(),tv.ravel()])
np.savez_compressed(p/'tracked-fixed.npz',xy=track,xyz=xyz,train=train,test=test,poses=poses,K=K,dist=dist)
print('tracks',len(xyz),'surviving',len(ids),'heldout',len(test),flush=True)
img=cv2.cvtColor(frames[0],cv2.COLOR_GRAY2BGR)
for i in ids:cv2.circle(img,tuple(np.round(track[0,i]).astype(int)),3,(0,255,255) if i in train else (255,0,255),-1)
cv2.imwrite(str(p/'tracks-dense.jpg'),img)
