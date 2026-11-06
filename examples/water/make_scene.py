"""Build the authored water experiment. Numerics and schedule come from libraries.fluid."""
import argparse, json, math, os, struct, subprocess
from pathlib import Path
HERE=Path(__file__).resolve().parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--vork',default=os.environ.get('VORK','vork'),help='built vork executable for standard mesh builders')
args=parser.parse_args()
import sys
sys.path.insert(0,str(HERE.parents[1]))
from libraries.fluid import build
metres_per_unit=1/6
# One authored cross-section drives both rendering and collision, including bevels.
BOWL_PROFILE=[[0,-.93],[.88,-.93],[.90,-.91],[.90,-.26],[.88,-.24],[.84,-.24],[.83,-.26],[.83,-.87],[0,-.87]]
TABLE_TOP=BOWL_PROFILE[0][1]
profile_source=f'static const float TABLE_TOP={TABLE_TOP};\nstatic const float BOWL_RADIUS={max(p[0] for p in BOWL_PROFILE)};\nstatic const uint BOWL_PROFILE_COUNT={len(BOWL_PROFILE)}u;\nstatic const float2 BOWL_PROFILE[{len(BOWL_PROFILE)}]={{'+','.join(f'float2({x},{y})' for x,y in BOWL_PROFILE)+'};\n'
fluid=build(callbacks=profile_source+(HERE/'bowl.slang').read_text(), extent=[.5,1/6,.5],
 scale=metres_per_unit, particle_budget=[128,45,128], spacing=.013/6,
 grid=[336,112,336], surface_grid=[768,256,768], bins=[192,64,192],
 film_grid=[512,1,512], dt=1/960, speed=.7, whitewater_budget=8192)
buffers=fluid['buffers'];programs=fluid['programs']
# Quaternions rotate -Z toward the target.
def look(p,t):
 import numpy as np
 z=np.array(p,dtype=float)-np.array(t,dtype=float);z/=np.linalg.norm(z);x=np.cross([0,1,0],z);x/=np.linalg.norm(x);y=np.cross(z,x);m=np.array([x,y,z]).T
 w=math.sqrt(1+np.trace(m))/2
 return [(m[2,1]-m[1,2])/(4*w),(m[0,2]-m[2,0])/(4*w),(m[1,0]-m[0,1])/(4*w),w]
materials={
 'water':dict(inputs=dict(base_weight=0,transmission_weight=1,specular_ior=1.333,specular_roughness=.005,transmission_color=[.78,.94,.98],transmission_depth=1)),
 'floor':dict(program='table',inputs=dict(wetness=dict(grid='wetness'))),
 'rim':dict(program='ceramic')}
materials.update(fluid['materials'])
materials['whitewater']=dict(inputs=dict(base_color=[.92,.96,1.],specular_roughness=.18))
programs['table']=dict(file='table.slang');programs['ceramic']=dict(file='ceramic.slang')
nodes=[dict(type='mesh',name='water',geometry='water',materials=['water'],scale=[3.,1,3.],translation=[0,0,0]),dict(type='mesh',name='basin',geometry='basin',materials=['rim']),dict(type='mesh',name='floor',geometry='box',materials=['floor'],translation=[0,TABLE_TOP-.1,0],scale=[200,.1,200]),dict(type='camera',name='camera',translation=[2.55,1.65,3.5],rotation=look([2.55,1.65,3.5],[0,-.18,0]),material=dict(program="std/camera",inputs=dict(lens=50,clip_start=.05,clip_end=100)))]
nodes.extend([dict(type='light',name='environment',shape='environment',material=dict(program='std/environment_image',inputs=dict(image=dict(image='environment'))),energy=.65),
 dict(type='light',name='sun',shape='sun',energy=2.5,angle=.015,rotation=[-.3826834,0,0,.9238795],color=[1,.94,.83])])
nodes.append(dict(type='mesh',name='floor_film',geometry='floor_film',materials=['water'],scale=[3.,2.,3.],translation=[0,TABLE_TOP,0]))
nodes.append(dict(type='mesh',name='whitewater',geometry='whitewater',materials=['whitewater']))
geometry=dict(fluid['geometry'])
geometry.update(basin=dict(std='revolve',profile=BOWL_PROFILE,segments=128),box=dict(std='box'))
scene=dict(version=2,images=dict(environment=dict(file='../assets/forest_slope_1k.hdr')),buffers=buffers,
 geometry=geometry,programs=programs,materials=materials,nodes=nodes,simulation=fluid['simulation'],
 render=dict(camera='camera',resolution=[1920,1080],fps=24,duration=1.5,motion_blur=dict(shutter=.5),background=[.09,.14,.21]))
# Convert authored solver-space geometry to SI exactly once. Camera distances and
# absorption remain physical; changing scale must never retime playback.
for node in nodes:
 if node['type'] in ('mesh','camera'):
  node['translation']=[v*metres_per_unit for v in node.get('translation',[0,0,0])]
  if node['type']=='mesh':
   node['scale']=[v*metres_per_unit for v in node.get('scale',[1,1,1])]
# Lower author-side builders to ordinary buffer geometry, like the other examples.
mesh_bytes=bytearray()
for name,definition in list(scene['geometry'].items()):
 if 'std' not in definition: continue
 options={k:v for k,v in definition.items() if k!='std'};options['name']='shape'
 fragment=json.loads(subprocess.check_output([args.vork,'std',definition['std'],json.dumps(options)],text=True))
 mesh=fragment['geometry']['shape']
 def pack(reference):
  source=fragment['buffers'][reference];data=source['data'];kind=source['type']
  flat=[component for value in data for component in (value if isinstance(value,list) else [value])]
  result=dict(type=kind,file='scene.std.bin',offset=len(mesh_bytes),count=len(data))
  mesh_bytes.extend(struct.pack('<'+str(len(flat))+('I' if kind=='u32' else 'f'),*flat))
  return result
 mesh['attributes']={semantic:pack(reference) for semantic,reference in mesh['attributes'].items()}
 if 'indices' in mesh:mesh['indices']=pack(mesh['indices'])
 scene['geometry'][name]=mesh
(HERE/'scene.std.bin').write_bytes(mesh_bytes)
import re
text=json.dumps(scene,indent=2)
text=re.sub(r'\[\s*([-+0-9.eE]+(?:,\s*[-+0-9.eE]+)*)\s*\]',lambda m: '['+re.sub(r'\s+','',m[1])+']',text)
text=re.sub(r'\[\s*(\[[-+0-9.eE,]+\](?:,\s*\[[-+0-9.eE,]+\])*)\s*\]',lambda m: '['+re.sub(r'\s+','',m[1])+']',text)
(HERE/'scene.json').write_text(text+'\n')
