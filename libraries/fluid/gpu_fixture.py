"""Small physical fixtures for the renderer's real-GPU fluid contracts."""
import json
import sys
from libraries.fluid import build

kind=sys.argv[1]
n=int(sys.argv[2])
callbacks='''
float container(float3 p) {return -10.;}
float initial_field(float3 p) {return 1.;}
bool initial_marker(uint id,out float3 p) {
 uint3 c=uint3(id%markers_dims.x,(id/markers_dims.x)%markers_dims.y,id/(markers_dims.x*markers_dims.y));
 p=(float3(c)-.5*(float3(markers_dims)-1.))*MARKER_SPACING;return true;
}
float3 initial_velocity(float3 p,float dx) {return 0.;}
float initial_pressure(float3 p) {return 0.;}
bool film_accept(float2 p) {return true;}
float film_height(float2 p) {return 0.;}
float3 boundary_velocity(float3 p,float time,float3 v) {return v;}
'''
scene=build(callbacks=callbacks,extent=[1,.5,1.5],particle_budget=[1,1,1] if kind=='sparse' else [8,1,8],
            spacing=.12 if kind=='sparse' else .08,grid=[16,8,24],surface_grid=[n,n//2,n*3//2],
            film_grid=[4,1,6],gravity=0,surface_tension=0,speed=1,dt=.001)
if kind=='dam':
    callbacks='''
float container(float3 p) {return max(-.9-p.y,max(abs(p.x)-1.9,abs(p.z)-1.9));}
float initial_field(float3 p) {return max(container(p),max(max(abs(p.x)-.8,abs(p.z)-.8),max(-.8-p.y,p.y-.16)));}
bool initial_marker(uint id,out float3 p) {
 uint3 c=uint3(id%markers_dims.x,(id/markers_dims.x)%markers_dims.y,id/(markers_dims.x*markers_dims.y));
 p=float3(-.8,-.8,-.8)+(float3(c)+.5)*MARKER_SPACING;return true;
}
float3 initial_velocity(float3 p,float dx) {return float3(1.5,0,0);}
float initial_pressure(float3 p) {return initial_field(p)<0. ? GRAVITY*max(0.,.16-p.y) : 0.;}
bool film_accept(float2 p) {return false;}
float film_height(float2 p) {return -.9;}
float3 boundary_velocity(float3 p,float time,float3 v) {return v;}
'''
    scene=build(callbacks=callbacks,extent=[2,1,2],particle_budget=[40,24,40],spacing=.04,
                grid=[64,32,64],surface_grid=[n,n//2,n],whitewater_budget=0,
                speed=8.,dt=1/480,flip=.9)
if kind in ('edge','corner','interior','edge_thin'):
    values=[0.]*24
    values[{'edge':8,'corner':0,'interior':10,'edge_thin':8}[kind]]=1e-8 if kind=='edge_thin' else .04
    scene['buffers']['wetness']['data']=values
    scene['simulation']['init']=['film_publish']
    scene['simulation']['steps']=['film_publish']
    scene['programs']={'film_publish':scene['programs']['film_publish']}
    scene['materials']={'film_publish':scene['materials']['film_publish']}
    scene['geometry']={'floor_film':scene['geometry']['floor_film']}
scene.update(version=2,nodes=[
    dict(type='mesh',geometry='floor_film' if kind in ('edge','corner','interior','edge_thin') else 'water',materials=['water'],scale=[1,1 if kind in ('edge','corner','interior','edge_thin') else .5,1.5]),
    dict(type='camera',name='camera',translation=[2,2,3],target=[0,0,0],material=dict(program='std/camera',inputs=dict(lens=50)))],
    render=dict(camera='camera',resolution=[32,32],background=[0,0,0]))
scene['materials']['water']=dict(inputs=dict(base_weight=0,specular_weight=0,emission_luminance=1,emission_color=[1,0,0]))
# Inline the selected checkout's library sources so the same GPU test binary
# can compare the audited and repaired numerics without changing renderer code.
from pathlib import Path
for program in scene['programs'].values():
    for module in ('config','solver','surface','film','whitewater'):
        program['source']=program['source'].replace('#include "fluid/'+module+'.slang"',Path(__file__).with_name(module+'.slang').read_text())
if kind=='dam':
    scene['nodes'][0]['scale']=[2,1,2]
print(json.dumps(scene))
