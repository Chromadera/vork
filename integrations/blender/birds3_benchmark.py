# SPDX-License-Identifier: GPL-3.0-or-later
"""Measurement adapter only. Actual vork checkpoints -> shared Blender GN instances."""
import bpy,numpy as np,json,math,time,sys,os
from pathlib import Path
from mathutils import Vector,Quaternion,Matrix
ROOT=Path(os.environ['VORK_BIRD_BENCH_ROOT']).resolve();OUT=ROOT/'results';os.chdir(ROOT)
SHOT=json.loads((ROOT/'shot.json').read_text());TICKS=SHOT['ticks']
S=json.loads((ROOT/SHOT['scene']).read_text())
MODE=sys.argv[sys.argv.index('--')+1]
LOG=OUT/(MODE+'-events.jsonl')
def event(kind,**kw):
    row=dict(kind=kind,**kw);print('BIRDBENCH '+json.dumps(row),flush=True)
    with LOG.open('a') as f:f.write(json.dumps(row)+'\n')
def cv(a):
    a=np.array(a,dtype=np.float32);return np.stack([a[...,0],-a[...,2],a[...,1]],axis=-1)
def buf(b):
    if isinstance(b,str):b=S['buffers'][b]
    if 'data' in b:return np.array(b['data'],dtype=np.uint32 if b['type']=='u32' else np.float32)
    w={'vec3':3,'vec4':4,'vec2':2,'u32':1,'f32':1}[b['type']]
    return np.fromfile(ROOT/'wetland'/b['file'],dtype=np.uint32 if b['type']=='u32' else np.float32,count=b['count']*w,offset=b.get('offset',0)).reshape(-1,w)
def quat(q):return Quaternion((q[3],q[0],-q[2],q[1]))
def eulers(q):
    x,y,z,w=q[:,0],-q[:,2],q[:,1],q[:,3]
    return np.stack([np.arctan2(2*(w*x+y*z),1-2*(x*x+y*y)),np.arcsin(np.clip(2*(w*y-z*x),-1,1)),np.arctan2(2*(w*z+x*y),1-2*(y*y+z*z))],axis=1).astype(np.float32)
def mesh(name,material):
    g=S['geometry'][name];v=cv(buf(g['attributes']['POSITION']));i=buf(g['indices']).reshape(-1,3)
    m=bpy.data.meshes.new(name);m.from_pydata(v.tolist(),[],i.tolist());m.update()
    if 'TEXCOORD_0' in g['attributes']:
        uv=buf(g['attributes']['TEXCOORD_0']);layer=m.uv_layers.new();lu=uv[i.reshape(-1)].copy();lu[:,1]=1-lu[:,1];layer.data.foreach_set('uv',lu.ravel())
    if 'NORMAL' in g['attributes']:
        for poly in m.polygons:poly.use_smooth=True
        m.normals_split_custom_set_from_vertices(cv(buf(g['attributes']['NORMAL'])).tolist())
    m.materials.append(material)
    obj=bpy.data.objects.new(name,m);bpy.context.scene.collection.objects.link(obj)
    return obj

def material(name,color,roughness,albedo=None,normal=None,alpha=False):
    m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;p=n.get('Principled BSDF');p.inputs['Base Color'].default_value=(*color,1);p.inputs['Roughness'].default_value=roughness
    if albedo:
        tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(ROOT/'wetland'/S['images'][albedo]['file']));l.new(tex.outputs['Color'],p.inputs['Base Color'])
        if alpha:
            cut=n.new('ShaderNodeMath');cut.operation='GREATER_THAN';cut.inputs[1].default_value=.4;l.new(tex.outputs['Alpha'],cut.inputs[0]);l.new(cut.outputs[0],p.inputs['Alpha'])
    if normal:
        tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(ROOT/'wetland'/S['images'][normal]['file']));tex.image.colorspace_settings.name='Non-Color';nm=n.new('ShaderNodeNormalMap');l.new(tex.outputs['Color'],nm.inputs['Color']);l.new(nm.outputs['Normal'],p.inputs['Normal'])
    return m

def instancer(name,source,p,q,scale,velocity):
    m=bpy.data.meshes.new(name);m.vertices.add(len(p));m.vertices.foreach_set('co',cv(p).ravel());m.update()
    for key,a in [('rot',eulers(q)),('scale',np.asarray(scale,dtype=np.float32)),('velocity',cv(velocity))]:
        attr=m.attributes.new(name=key,type='FLOAT_VECTOR',domain='POINT');attr.data.foreach_set('vector',np.asarray(a,dtype=np.float32).ravel())
    obj=bpy.data.objects.new(name,m);bpy.context.scene.collection.objects.link(obj)
    mod=obj.modifiers.new('Shared instances','NODES');g=bpy.data.node_groups.new(name,'GeometryNodeTree');mod.node_group=g
    g.interface.new_socket(name='Geometry',in_out='INPUT',socket_type='NodeSocketGeometry');g.interface.new_socket(name='Geometry',in_out='OUTPUT',socket_type='NodeSocketGeometry')
    n=g.nodes;l=g.links;inp=n.new('NodeGroupInput');out=n.new('NodeGroupOutput');ins=n.new('GeometryNodeInstanceOnPoints');info=n.new('GeometryNodeObjectInfo');info.inputs['Object'].default_value=source;info.inputs['As Instance'].default_value=True
    l.new(info.outputs['Geometry'],ins.inputs['Instance']);l.new(ins.outputs['Instances'],out.inputs['Geometry'])
    for key,target in [('rot','Rotation'),('scale','Scale')]:
        a=n.new('GeometryNodeInputNamedAttribute');a.data_type='FLOAT_VECTOR';a.inputs['Name'].default_value=key;l.new(a.outputs['Attribute'],ins.inputs[target])
    # Linear subframe velocities allow motion blur without charging any solver.
    st=n.new('GeometryNodeInputSceneTime');delta=n.new('ShaderNodeMath');delta.operation='SUBTRACT';delta.name='snapshot_time';l.new(st.outputs['Seconds'],delta.inputs[0])
    vel=n.new('GeometryNodeInputNamedAttribute');vel.data_type='FLOAT_VECTOR';vel.inputs['Name'].default_value='velocity'
    mul=n.new('ShaderNodeVectorMath');mul.operation='SCALE';l.new(vel.outputs['Attribute'],mul.inputs[0]);l.new(delta.outputs[0],mul.inputs['Scale'])
    sp=n.new('GeometryNodeSetPosition');l.new(inp.outputs['Geometry'],sp.inputs['Geometry']);l.new(mul.outputs['Vector'],sp.inputs['Offset']);l.new(sp.outputs['Geometry'],ins.inputs['Points'])
    source.hide_render=True;source.hide_set(True)
    return obj

def tracks():
    for anim in S['animation']:
        for tr in anim['tracks']:
            path=tr['path'];keys=tr['keys'];name=path[1]
            if name not in bpy.data.objects:continue
            obj=bpy.data.objects[name]
            if name=='camera' and path[2]=='rotation':
                lens=bpy.data.objects['camera-lens']
                for t,v in keys:
                    lens.rotation_quaternion=Quaternion((v[3],v[0],v[1],v[2]));lens.keyframe_insert('rotation_quaternion',frame=t*24)
                continue
            if path[2] in ('translation','rotation'):
                prop='location' if path[2]=='translation' else 'rotation_quaternion';obj.rotation_mode='QUATERNION'
                for t,v in keys:
                    setattr(obj,prop,cv(v) if prop=='location' else quat(v));obj.keyframe_insert(prop,frame=t*24)
            elif path[2]=='morph_weights':
                key=obj.data.shape_keys.key_blocks[path[3]]
                for t,v in keys:key.value=v;key.keyframe_insert('value',frame=t*24)
    # Set every authored track to linear, consistent with the source.
    for ac in bpy.data.actions:
        for layer in ac.layers:
            for strip in layer.strips:
                for slot in ac.slots:
                    bag=strip.channelbag(slot)
                    if bag:
                        for fc in bag.fcurves:
                            for k in fc.keyframe_points:k.interpolation='LINEAR'
                            if fc.data_path.startswith('key_blocks['):fc.modifiers.new('CYCLES')

def setup():
    start=time.perf_counter();bpy.ops.wm.read_factory_settings(use_empty=True);sc=bpy.context.scene
    far=material('silhouette',(.018,.013,.022),.65);crow=material('crow',(.018,.013,.022),.62,'crow','crow_normal',True);hero=material('hero',(.018,.013,.022),.42,'starling','starling_normal')
    bird=mesh('silhouette',far);near=mesh('flight',crow);body=mesh('hero',hero);wing=mesh('wings',crow)
    bird.hide_render=near.hide_render=body.hide_render=wing.hide_render=True
    for i in range(9):
        parent=bpy.data.objects.new(f'hero-{i}',None);sc.collection.objects.link(parent)
        for source,suffix in [(body,'body'),(wing,'wings')]:
            obj=bpy.data.objects.new(f'hero-{i}-{suffix}',source.data if suffix=='body' else source.data.copy());sc.collection.objects.link(obj);obj.parent=parent
            if suffix=='wings':
                obj.shape_key_add(name='Basis')
                for key,b in S['geometry']['wings']['morphs'].items():
                    sk=obj.shape_key_add(name=key);sk.data.foreach_set('co',(cv(buf(S['geometry']['wings']['attributes']['POSITION']))+cv(buf(b['POSITION']))).ravel())
    water=material('water',(.018,.027,.032),.065);water.node_tree.nodes.get('Principled BSDF').inputs['IOR'].default_value=1.333;mesh('water',water)
    camera=bpy.data.objects.new('camera',None);sc.collection.objects.link(camera);lens=bpy.data.objects.new('camera-lens',bpy.data.cameras.new('camera-lens'));sc.collection.objects.link(lens);lens.parent=camera;lens.rotation_mode='QUATERNION';sc.camera=lens;lens.data.lens=30;lens.data.sensor_width=36;lens.data.clip_start=.1;lens.data.clip_end=500
    target=bpy.data.objects.new('aim',None);sc.collection.objects.link(target);con=camera.constraints.new('TRACK_TO');con.target=target;con.track_axis='TRACK_NEGATIVE_Z';con.up_axis='UP_Y'
    world=bpy.data.worlds.new('dusk');world.use_nodes=True;sc.world=world;n=world.node_tree.nodes;l=world.node_tree.links
    env=n.new('ShaderNodeTexEnvironment');env.image=bpy.data.images.load(str(ROOT/'wetland/assets/sky.hdr'));env.image.colorspace_settings.name='Linear Rec.709';coord=n.new('ShaderNodeTexCoord');mapping=n.new('ShaderNodeMapping');mapping.inputs['Rotation'].default_value[2]=-2*math.atan2(.9377,.3474);l.new(coord.outputs['Generated'],mapping.inputs['Vector']);l.new(mapping.outputs['Vector'],env.inputs['Vector']);l.new(env.outputs['Color'],n['Background'].inputs['Color']);n['Background'].inputs['Strength'].default_value=.18
    # Original sun and separate fill sun, including parent world rotation.
    for node in S['nodes']:
        lights=[(node,None)] if node.get('shape')=='sun' else [(child,node) for child in node.get('children',[]) if child.get('shape')=='sun']
        for light,parent in lights:
            data=bpy.data.lights.new(light['name'],'SUN');data.energy=light['energy']*(parent.get('energy',1) if parent else 1);data.color=light['color'];data.angle=light.get('angle',.00918);o=bpy.data.objects.new(light['name'],data);sc.collection.objects.link(o);o.rotation_mode='QUATERNION';o.rotation_quaternion=quat(light['rotation'])
            if parent:o.rotation_quaternion=quat(parent['rotation'])@o.rotation_quaternion
    tracks();sc.render.fps=24;sc.render.resolution_x=1920;sc.render.resolution_y=1080;sc.render.resolution_percentage=100
    sc.render.engine='BLENDER_EEVEE';sc.eevee.taa_render_samples=16;sc.eevee.use_raytracing=True
    sc.render.use_motion_blur=True;sc.render.motion_blur_shutter=.2;sc.render.film_transparent=False;sc.render.use_compositing=False
    sc.render.image_settings.file_format='PNG';sc.render.image_settings.color_mode='RGB';sc.view_settings.view_transform='AgX';sc.view_settings.look='None';sc.render.dither_intensity=0
    state=dict(np.load(ROOT/f'state-{TICKS[0]}.npz'));state['near_scale'][int(state['near_count'][0,0]):]=0;instancer('flock',bird,state['translation'],state['rotation'],state['far_scale'],state['v'])
    # Constant capacity keeps GN topology stable for subframe evaluation.
    instancer('near-flock',near,state['near_translation'],state['near_rotation'],state['near_scale'],near_vel(state))
    update(TICKS[0])
    event('setup',elapsed_s=time.perf_counter()-start,blender_version=bpy.app.version_string,far_instances=1000000,far_triangles=len(bird.data.polygons),near_capacity=32768,near_triangles=len(near.data.polygons),heroes=9,hero_triangles=len(body.data.polygons),wing_triangles=len(wing.data.polygons))
    start=time.perf_counter();bpy.ops.wm.save_as_mainfile(filepath=str(ROOT/'birds.blend'),compress=False);event('save',elapsed_s=time.perf_counter()-start,bytes=(ROOT/'birds.blend').stat().st_size)

def near_vel(state):
    out=np.zeros((32768,3),np.float32);n=int(state['near_count'][0,0]);ids=np.flatnonzero(state['far_scale'][:,0]==0);k=min(len(ids),n);out[:k]=state['v'][ids[:k]];return out

def update(tick):
    t0=time.perf_counter();state=dict(np.load(ROOT/f'state-{tick}.npz'));state['near_scale'][int(state['near_count'][0,0]):]=0;t=tick/60
    for name,p,q,scale,velocity in [('flock',state['translation'],state['rotation'],state['far_scale'],state['v']),('near-flock',state['near_translation'],state['near_rotation'],state['near_scale'],near_vel(state))]:
        obj=bpy.data.objects[name];m=obj.data;m.vertices.foreach_set('co',cv(p).ravel())
        for key,a in [('rot',eulers(q)),('scale',scale),('velocity',cv(velocity))]:m.attributes[key].data.foreach_set('vector',np.asarray(a,dtype=np.float32).ravel())
        m.update();obj.modifiers[0].node_group.nodes['snapshot_time'].inputs[1].default_value=t
    frame=t*24;bpy.context.scene.frame_set(math.floor(frame),subframe=frame%1)
    return time.perf_counter()-t0

def render():
    t=time.perf_counter();bpy.ops.wm.open_mainfile(filepath=str(ROOT/'birds.blend'),load_ui=False);event('load',elapsed_s=time.perf_counter()-t)
    sc=bpy.context.scene
    if MODE=='cycles':
        sc.render.engine='CYCLES';sc.cycles.device='GPU';prefs=bpy.context.preferences.addons['cycles'].preferences;prefs.compute_device_type='OPTIX';prefs.get_devices()
        for d in prefs.devices:d.use=d.type=='OPTIX'
        sc.cycles.samples=16;sc.cycles.use_adaptive_sampling=False;sc.cycles.use_denoising=True;sc.render.use_persistent_data=True
        event('devices',devices=[dict(name=d.name,type=d.type,use=d.use) for d in prefs.devices]);groups=[(16,[TICKS[0],TICKS[0],TICKS[len(TICKS)//2],TICKS[-1]])]
    elif MODE=='eevee-previs':
        sc.render.engine='BLENDER_EEVEE';sc.eevee.use_raytracing=False;groups=[(4,[TICKS[0]]+TICKS)]
    else:groups=[(16,[TICKS[0]]+TICKS)]+([(4,TICKS)] if SHOT['name']=='skim' else [])
    event('settings',engine=sc.render.engine,resolution=[sc.render.resolution_x,sc.render.resolution_y],motion_blur=sc.render.use_motion_blur,shutter=sc.render.motion_blur_shutter,raytracing=sc.eevee.use_raytracing,compositor=sc.render.use_compositing)
    for samples,ticks in groups:
        sc.eevee.taa_render_samples=samples
        for index,tick in enumerate(ticks):
            start=time.perf_counter();update_s=update(tick);sc.render.filepath=str(OUT/(f'{MODE}-{samples}-{tick}'+('-cold.png' if index==0 and (samples==16 or MODE=='eevee-previs') else '.png')));tr=time.perf_counter();bpy.ops.render.render(write_still=True)
            event('frame',engine=MODE,samples=samples,index=index,tick=tick,source_time=tick/60,cold=(index==0 and (samples==16 or MODE=='eevee-previs')),update_s=update_s,render_and_png_s=time.perf_counter()-tr,total_s=time.perf_counter()-start)
if MODE=='setup':setup()
else:render()
