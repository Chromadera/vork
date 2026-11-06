"""Camera reconstruction stage; invoked by camera.py, outputs outside git."""
from pathlib import Path
import pycolmap
import os
root=Path(os.environ['VFX_NEON_EVIDENCE'])
db=root/'solve.db'
with pycolmap.Database.open(db) as d:
 c=d.read_camera(1);c.params=[1500,960,412.5,0];c.has_prior_focal_length=True;d.update_camera(c)
out=root/'fixed-sparse';out.mkdir(exist_ok=True)
opt=pycolmap.IncrementalPipelineOptions();opt.num_threads=8;opt.ba_refine_focal_length=False;opt.ba_refine_extra_params=False;opt.ba_refine_principal_point=False;opt.mapper.init_min_tri_angle=2
recs=pycolmap.incremental_mapping(db,root/'frames',out,options=opt)
print([(k,r.summary(),r.cameras) for k,r in recs.items()])
