# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline asset build: blender -b --python prepare_birds.py -- SOURCE OUTPUT.

SOURCE contains the licensed starling/ and crow/ glTF downloads. Blender 5.2.1.
Outputs native vertex streams and small textures; no runtime Blender dependency.
"""
import sys, json, math, struct
from pathlib import Path
import bpy
from mathutils import Vector, Matrix
source, output = map(Path, sys.argv[sys.argv.index('--')+1:])
output.mkdir(parents=True, exist_ok=True)
buffers, geometries = {}, {}
blob = bytearray()
def stream(name, ty, values):
    flat = [x for v in values for x in v] if ty != 'u32' else values
    offset=len(blob);blob.extend(struct.pack('<'+('I' if ty=='u32' else 'f')*len(flat),*flat))
    buffers[name]={'type':ty,'file':'assets/birds/birds.bin','offset':offset,'count':len(values)}
    return name

def emit(obj,name,convert,filter_face=lambda p:True,largest=False):
    mesh=obj.data;mesh.calc_loop_triangles();positions=[];normals=[];uvs=[];indices=[];lookup={}
    triangles=[]
    for tri in mesh.loop_triangles:
        ps=[obj.matrix_world @ mesh.vertices[mesh.loops[l].vertex_index].co for l in tri.loops]
        if filter_face(sum(ps,Vector())/3):triangles.append((tri,ps))
    if largest:
        # Cropping the perch can leave isolated scan fragments. Keep the bird's
        # connected surface, using source vertex identity across UV seams.
        parent=list(range(len(mesh.vertices)))
        def find(v):
            while parent[v]!=v:
                parent[v]=parent[parent[v]];v=parent[v]
            return v
        for tri,_ in triangles:
            vs=list(tri.vertices)
            for v in vs[1:]:parent[find(v)]=find(vs[0])
        sizes={}
        for tri,_ in triangles:
            key=find(tri.vertices[0]);sizes[key]=sizes.get(key,0)+1
        keep=max(sizes,key=sizes.get)
        triangles=[(t,p) for t,p in triangles if find(t.vertices[0])==keep]
    for tri,ps in triangles:
        for l,p in zip(tri.loops,ps):
            n=obj.matrix_world.to_3x3() @ mesh.corner_normals[l].vector
            p,n=convert(p,n);uv0=mesh.uv_layers.active.data[l].uv if mesh.uv_layers else (0,0)
            uv=(uv0[0],1.-uv0[1])
            key=tuple(p)+tuple(n)+tuple(uv)
            if key not in lookup:
                lookup[key]=len(positions);positions.append(tuple(p));normals.append(tuple(n));uvs.append(tuple(uv))
            indices.append(lookup[key])
    geometries[name]={'attributes':{'POSITION':stream(name+'_p','vec3',positions),'NORMAL':stream(name+'_n','vec3',normals),'TEXCOORD_0':stream(name+'_uv','vec2',uvs)},'indices':stream(name+'_i','u32',indices)}
    if name == 'wings':
        morphs={}
        for pose,angle in [('upstroke',.8),('downstroke',-.65)]:
            delta=[];normal_delta=[]
            for p,n in zip(positions,normals):
                span=max(0.,abs(p[0])-.035)
                side=1. if p[0]>=0 else -1.
                delta.append((side*span*(math.cos(angle)-1),span*math.sin(angle),-.012*min(span/.13,1)))
                a=side*angle
                rotated=(n[0]*math.cos(a)-n[1]*math.sin(a),n[0]*math.sin(a)+n[1]*math.cos(a),n[2])
                normal_delta.append(tuple(rotated[k]-n[k] for k in range(3)))
            morphs[pose]={'POSITION':stream(name+'_'+pose,'vec3',delta),
                          'NORMAL':stream(name+'_'+pose+'_n','vec3',normal_delta)}
        geometries[name]['morphs']=morphs
    print('OUTPUT',name,len(indices)//3,'triangles',flush=True)

# Flight pose, after the perched part of TakeOff. Collapse the rig into rest geometry.
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source/'crow/scene.gltf'))
bpy.context.scene.frame_set(210)
crow=next(o for o in bpy.context.scene.objects if o.type=='MESH' and len(o.data.vertices)>100)
bpy.context.view_layer.objects.active=crow;crow.select_set(True)
dg=bpy.context.evaluated_depsgraph_get();mesh=bpy.data.meshes.new_from_object(crow.evaluated_get(dg))
obj=bpy.data.objects.new('flight',mesh);bpy.context.collection.objects.link(obj);obj.matrix_world=crow.matrix_world.copy()
ps=[obj.matrix_world @ v.co for v in mesh.vertices]
print('CROW BOUNDS',[(min(p[k] for p in ps),max(p[k] for p in ps)) for k in range(3)],flush=True)
# Imported glTF is Y-up, Blender is Z-up. Centre at the shoulder, scale to 42 cm wingspan.
lo=[min(p[k] for p in ps) for k in range(3)];hi=[max(p[k] for p in ps) for k in range(3)]
center=Vector(((lo[0]+hi[0])/2,(lo[1]+hi[1])/2,(lo[2]+hi[2])/2));scale=.42/(hi[0]-lo[0])
def crow_space(p,n):
    q=(p-center)*scale
    return Vector((q.x,q.z,-q.y)),Vector((n.x,n.z,-n.y)).normalized()
emit(obj,'flight',crow_space)
def hero_wing_space(p,n):
    q,r=crow_space(p,n)
    return q+Vector((0,-.018,.015)),r
emit(obj,'wings',hero_wing_space,lambda p:abs((p-center).x)*scale>.035)
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source/'starling/scene.gltf'))
bpy.ops.object.select_all(action='DESELECT')
for o in bpy.context.scene.objects:
    if o.type=='MESH' and o.name!='Object_17':o.select_set(True);bpy.context.view_layer.objects.active=o
bpy.ops.object.join();high=bpy.context.object
bpy.ops.object.transform_apply(location=True,rotation=True,scale=True)
# Weld glTF primitive boundaries before decimation; preserve UV corner seams.
bpy.ops.object.mode_set(mode='EDIT');bpy.ops.mesh.select_all(action='SELECT');bpy.ops.mesh.remove_doubles(threshold=.000001);bpy.ops.object.mode_set(mode='OBJECT')
low=high.copy();low.data=high.data.copy();bpy.context.collection.objects.link(low);bpy.context.view_layer.objects.active=low
mod=low.modifiers.new('hero budget','DECIMATE');mod.ratio=.024;bpy.ops.object.modifier_apply(modifier=mod.name)
# Bake scan surface normals onto the reduced UV mesh, selected to active.
image=bpy.data.images.new('scan baked normals',1024,1024,alpha=False)
# Shared materials use the same target image; no surface input consumes it during baking.
for mat in set(low.data.materials):
    node=mat.node_tree.nodes.new('ShaderNodeTexImage');node.image=image;mat.node_tree.nodes.active=node
bpy.context.scene.render.engine='CYCLES';bpy.context.scene.cycles.samples=1
bpy.context.scene.render.bake.use_selected_to_active=True;bpy.context.scene.render.bake.cage_extrusion=.003
bpy.ops.object.select_all(action='DESELECT');high.select_set(True);low.select_set(True)
bpy.ops.object.bake(type='NORMAL')
image.filepath_raw=str(output/'starling-normal.png');image.file_format='PNG';image.save()
rot=Matrix.Rotation(math.radians(-51),3,'X');origin=Vector((.014,-.015,.245))
def scan_space(p,n):
    q=rot @ (p-origin);r=rot @ n
    return Vector((-q.x,q.z,q.y)),Vector((-r.x,r.z,r.y)).normalized()
# The scan includes a perch. Keep the torso/head and rear tail, cutting away perch/extended feet.
emit(low,'hero',scan_space,lambda p:p.z>.20 or (p.y<-.035 and p.z>.09),largest=True)
(output/'birds.bin').write_bytes(blob)
(output/'birds.json').write_text(json.dumps({'buffers':buffers,'geometry':geometries},indent=2)+'\n')
# Keep production textures small while retaining the crow's feather alpha.
for relative,name,size in [('crow/textures/M_Crow_baseColor.png','crow.png',1024),
                           ('crow/textures/M_Crow_normal.png','crow-normal.png',512),
                           ('starling/textures/szpak-mat_diffuse.jpeg','starling.jpg',1024)]:
    im=bpy.data.images.load(str(source/relative),check_existing=False)
    factor=size/max(im.size);im.scale(round(im.size[0]*factor),round(im.size[1]*factor))
    im.filepath_raw=str(output/name);im.file_format='JPEG' if name.endswith('.jpg') else 'PNG';im.save()
image.scale(512,512);image.save()
