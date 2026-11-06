"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
from pathlib import Path
import pycolmap,subprocess,cv2,numpy as np
import os
p=Path(os.environ['VFX_NEON_EVIDENCE'])
frames=p/'frames';frames.mkdir(exist_ok=True);masks=p/'masks';masks.mkdir(exist_ok=True)
subprocess.run(['ffmpeg','-v','error','-y','-i',str(Path(os.environ['VFX_NEON_MEDIA'])/'source.mp4'),'-t','6','-vf','fps=8,scale=1920:825',str(frames/'%04d.png')],check=True)
# Only building/fence surfaces. Exclude ground, car, lamps and sky.
for f in sorted(frames.glob('*.png')):
 im=cv2.imread(str(f));mask=np.zeros(im.shape[:2],np.uint8)
 cv2.fillPoly(mask,[np.array([[90,180],[740,270],[780,470],[290,735],[100,700]])],255)
 cv2.fillPoly(mask,[np.array([[1110,290],[1220,290],[1235,510],[1110,460]])],255)
 cv2.imwrite(str(masks/(f.name+'.png')),mask)
reader=pycolmap.ImageReaderOptions();reader.camera_model='SIMPLE_RADIAL';reader.mask_path=str(masks);reader.camera_params='1500,960,412.5,0'
ex=pycolmap.FeatureExtractionOptions();ex.num_threads=8;ex.sift.max_num_features=10000;ex.sift.peak_threshold=.002
pycolmap.extract_features(p/'solve.db',frames,camera_mode=pycolmap.CameraMode.SINGLE,reader_options=reader,extraction_options=ex,device=pycolmap.Device.cpu)
ma=pycolmap.FeatureMatchingOptions();ma.num_threads=8
pair=pycolmap.SequentialPairingOptions();pair.overlap=12
pycolmap.match_sequential(p/'solve.db',matching_options=ma,pairing_options=pair,device=pycolmap.Device.cpu)
out=p/'sparse';out.mkdir(exist_ok=True)
opt=pycolmap.IncrementalPipelineOptions();opt.num_threads=8;opt.ba_refine_principal_point=False;opt.mapper.init_min_tri_angle=1
r=pycolmap.incremental_mapping(p/'solve.db',frames,out,options=opt)
print([(k,v.summary()) for k,v in r.items()],flush=True)
