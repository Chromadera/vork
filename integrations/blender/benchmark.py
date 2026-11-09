# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender 5.2.1 first-frame baseline. Run with system Python; see --help."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time


def prepare(scene_path, out, engine, width, height, video=False):
    import bpy
    from mathutils import Vector
    bpy.ops.wm.read_factory_settings(use_empty=True)
    source = json.loads(scene_path.read_text())
    scene = bpy.context.scene
    objects = {}
    aims = []
    def point(v):
        return Vector((v[0], -v[2], v[1]))
    for n in source['nodes']:
        if n['type'] == 'asset':
            before = set(bpy.data.objects)
            bpy.ops.import_scene.gltf(filepath=str(scene_path.parent / n['file']))
            imported = set(bpy.data.objects) - before
            obj = bpy.data.objects.new(n['name'], None)
            scene.collection.objects.link(obj)
            for child in imported:
                if child.parent is None:
                    child.parent = obj
            objects[n['name']] = obj
            for attr, value in [('location', n.get('translation')), ('scale', n.get('scale'))]:
                if value is not None:
                    setattr(obj, attr, point(value) if attr == 'location' else (value[0], value[2], value[1]))
        elif n['type'] == 'camera':
            data = bpy.data.cameras.new(n['name'])
            data.lens = source['materials'][n['material']]['inputs']['lens']
            inputs = source['materials'][n['material']]['inputs']
            data.dof.use_dof = inputs.get('use_dof', False)
            data.dof.focus_distance = inputs.get('focus_distance', 10)
            data.dof.aperture_fstop = inputs.get('aperture_fstop', 2.8)
            data.clip_start = .01
            data.clip_end = 10000
            obj = bpy.data.objects.new(n['name'], data)
            scene.collection.objects.link(obj)
            obj.location = point(n['translation'])
            aims.append((obj, n['target']))
            scene.camera = obj
            objects[n['name']] = obj
        elif n['type'] == 'light' and n['shape'] == 'environment':
            world = bpy.data.worlds.new('world')
            world.use_nodes = True
            world.node_tree.nodes['Background'].inputs['Color'].default_value = (*n['color'], 1)
            world.node_tree.nodes['Background'].inputs['Strength'].default_value = n['energy']
            scene.world = world
        elif n['type'] == 'light':
            data = bpy.data.lights.new(n['name'], 'AREA' if n['shape'] == 'rectangle' else n['shape'].upper())
            if n['shape'] == 'rectangle':
                data.shape, data.size, data.size_y = 'RECTANGLE', n['size'], n.get('size_y', n['size'])
            data.energy = n['energy']
            data.color = n['color']
            if n['shape'] == 'point':
                data.shadow_soft_size = 0
            if n['shape'] == 'sun':
                data.angle = 0.00918043
            obj = bpy.data.objects.new(n['name'], data)
            scene.collection.objects.link(obj)
            obj.location = point(n['translation'])
            aims.append((obj, n['target']))
            objects[n['name']] = obj
    # Resolve node targets after imports and authored nodes exist, including
    # forward references. Track To follows animated origins throughout the movie.
    for obj, aim in aims:
        if isinstance(aim, str):
            target = objects[aim]
        else:
            target = bpy.data.objects.new(obj.name + '-target', None)
            scene.collection.objects.link(target)
            target.location = point(aim)
        constraint = obj.constraints.new('TRACK_TO')
        constraint.target = target
        constraint.track_axis, constraint.up_axis = 'TRACK_NEGATIVE_Z', 'UP_Y'
    scene.render.engine = engine
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.render.dither_intensity = 0
    scene.view_settings.view_transform = 'AgX'
    scene.render.fps = 24
    scene.frame_start, scene.frame_end = 0, 239
    for layer in source.get('animation', []):
        if layer.get('clip'):
            clip = layer['clip']
            asset = objects[clip['target']]
            # The glTF importer creates action slots and NLA tracks per animated object.
            # Select the requested source clip by its glTF animation name.
            import struct
            node = next(n for n in source['nodes'] if n.get('name') == clip['target'])
            raw = (scene_path.parent / node['file']).read_bytes()
            if raw[:4] == b'glTF':
                size = struct.unpack_from('<I', raw, 12)[0]
                gltf = json.loads(raw[20:20+size])
            else:
                gltf = json.loads(raw)
            animations = gltf.get('animations', [])
            wanted = clip.get('name') or animations[clip.get('index', 0)]['name']
            for child in asset.children_recursive:
                if child.animation_data:
                    for track in child.animation_data.nla_tracks:
                        track.mute = track.name != wanted
                    actions = [action for action in bpy.data.actions if action.name == wanted]
                    if actions:
                        child.animation_data.action = actions[0]
        for track in layer.get('tracks', []):
            path = track['path']
            if path[0] == 'nodes':
                obj = objects[path[1]]
                prop = {'translation':'location', 'scale':'scale', 'rotation':'rotation_quaternion'}[path[2]]
                if prop == 'rotation_quaternion':
                    obj.rotation_mode = 'QUATERNION'
                for t, value in track['keys']:
                    if prop == 'location': value = point(value)
                    elif prop == 'scale': value = (value[0], value[2], value[1])
                    elif prop == 'rotation_quaternion': value = (value[3], value[0], -value[2], value[1])
                    setattr(obj, prop, value)
                    obj.keyframe_insert(prop, frame=t*24)
            elif path[0] == 'materials':
                camera = next(n for n in source['nodes'] if n.get('material') == path[1] and n['type'] == 'camera')
                data = objects[camera['name']].data
                prop = {'lens':'lens', 'focus_distance':'dof.focus_distance', 'aperture_fstop':'dof.aperture_fstop'}[path[3]]
                if prop.startswith('dof.'):
                    data.dof.use_dof = True
                for t, value in track['keys']:
                    owner, leaf = (data.dof, prop[4:]) if prop.startswith('dof.') else (data, prop)
                    setattr(owner, leaf, value)
                    data.keyframe_insert(prop, frame=t*24)
    for action in bpy.data.actions:
        for layer in action.layers:
            for strip in layer.strips:
                for slot in action.slots:
                    bag = strip.channelbag(slot)
                    if bag:
                        for curve in bag.fcurves:
                            for key in curve.keyframe_points:
                                key.interpolation = 'LINEAR'
    if source['render'].get('motion_blur'):
        scene.render.use_motion_blur = True
        scene.render.motion_blur_shutter = source['render']['motion_blur'].get('shutter', .5)
    scene.frame_set(0)
    if engine == 'CYCLES':
        scene.cycles.device = 'GPU'
        scene.cycles.samples = 4096
        scene.cycles.use_adaptive_sampling = True
        scene.cycles.adaptive_threshold = .01
        scene.cycles.adaptive_min_samples = 64
        scene.cycles.use_denoising = False
        scene.cycles.max_bounces = 12
        scene.cycles.diffuse_bounces = scene.cycles.glossy_bounces = 4
        scene.cycles.transmission_bounces = 12
        scene.cycles.transparent_max_bounces = 8
        scene.cycles.sample_clamp_direct = 0
        scene.cycles.sample_clamp_indirect = 10
    elif engine == 'BLENDER_EEVEE':
        scene.eevee.taa_render_samples = 64
        scene.eevee.use_raytracing = True
    if video:
        scene.render.image_settings.media_type = 'VIDEO'
        scene.render.ffmpeg.format = 'MPEG4'
        scene.render.ffmpeg.codec = 'H264'
        scene.render.ffmpeg.constant_rate_factor = 'HIGH'
    bpy.ops.wm.save_as_mainfile(filepath=str(out))


def main():
    if '--prepare' in sys.argv:
        args = sys.argv[sys.argv.index('--prepare')+1:]
        prepare(Path(args[0]), Path(args[1]), args[2], int(args[3]), int(args[4]), len(args)>5 and args[5]=='video')
        return
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--blender', default='blender')
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--repetitions', type=int, default=5)
    p.add_argument('--resolution', default='1920x1080')
    p.add_argument('--cycles-device', choices=['OPTIX', 'CUDA', 'METAL'], default='OPTIX')
    p.add_argument('--scenes', help='Comma-separated scene names; default: every B-*.json')
    p.add_argument('--video', action='store_true', help='Render all 240 frames, including authored camera and asset animation')
    p.add_argument('--engines', default='BLENDER_EEVEE,CYCLES', help='Comma-separated BLENDER_EEVEE,CYCLES,BLENDER_WORKBENCH')
    a = p.parse_args()
    if a.out.exists():
        p.error('--out must be new')
    a.out = a.out.resolve()
    a.out.mkdir(parents=True)
    width, height = map(int, a.resolution.split('x'))
    root = Path(__file__).resolve().parents[2]
    (a.out/'version.txt').write_text(subprocess.check_output([a.blender,'--version'],text=True))
    rows=[]
    names = a.scenes.split(',') if a.scenes else sorted(path.stem for path in (root/'benchmarks').glob('B-*.json'))
    for name in names:
        for engine in a.engines.split(','):
            stem=name+'-'+engine
            blend=a.out/(stem+'.blend')
            cmd=[a.blender,'-b','--disable-autoexec','--python-exit-code','1','--python',str(Path(__file__).resolve()),
                 '--','--prepare',str(root/'benchmarks'/(name+'.json')),str(blend),engine,str(width),str(height),'video' if a.video else 'still']
            with (a.out/(stem+'-prepare.log')).open('w') as log:
                subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,check=True)
            for i in range(-1,a.repetitions):
                cmd=[a.blender,'-b','--disable-autoexec',str(blend),'-o',str(a.out/(stem+f'-{i}-')),*(['-s','0','-e','239','-a'] if a.video else ['-f','0'])]
                if engine=='CYCLES':cmd+=['--','--cycles-device',a.cycles_device]
                start=time.perf_counter()
                with (a.out/(stem+f'-{i}.log')).open('w') as log:
                    result=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
                rows.append(dict(scene=name,engine=engine,phase='warmup' if i==-1 else 'warm',seconds=time.perf_counter()-start,returncode=result.returncode,command=cmd))
                (a.out/'timings.json').write_text(json.dumps(rows,indent=2)+'\n')
                if result.returncode:raise SystemExit(result.returncode)

if __name__ == '__main__':
    main()
