# SPDX-License-Identifier: GPL-3.0-or-later
"""Sample Blender scene animation into glTF clips and native camera/light tracks."""
import json
from pathlib import Path
import struct
from types import SimpleNamespace
import bpy
from mathutils import Matrix
from .geometry import export_gltf
from .render_geometry import gltf_render_settings
from .pose import rotation as pose_rotation
from .camera import camera_fit
from .lights import color as light_color, energy as light_energy, temperature as light_temperature

_BASIS = Matrix(((1,0,0,0),(0,0,1,0),(0,-1,0,0),(0,0,0,1)))


def gltf_document(path):
    data = Path(path).read_bytes()
    if data[:4] == b'glTF':
        size, kind = struct.unpack_from('<II', data, 12)
        if kind != 0x4E4F534A:
            raise ValueError('GLB has no initial JSON chunk')
        return json.loads(data[20:20+size])
    return json.loads(data)


def export_clip(filepath, asset_name, *, apply_modifiers=True, **options):
    """Scene sampling retains constraints/drivers and a common timeline.

    Applying modifiers can remove shape keys. Inspect emitted targets per object
    and report missing morphs rather than claiming all were retained or dropped. Armatures use native glTF skin clips.
    """
    scene = bpy.context.scene
    original = scene.frame_current, scene.frame_subframe
    try:
        scene.frame_set(scene.frame_start)
        export_gltf(filepath=str(filepath), export_format='GLB', export_apply=apply_modifiers,
                    export_animations=True, export_force_sampling=True,
                    export_animation_mode='SCENE', export_anim_scene_split_object=False,
                    export_anim_slide_to_zero=True, export_frame_range=True,
                    export_current_frame=False, **options)
    finally:
        scene.frame_set(original[0], subframe=original[1])
    doc = gltf_document(filepath)
    clips = doc.get('animations', [])
    channels = [c for clip in clips for c in clip.get('channels', [])]
    findings = []
    morph_objects = {node.get('name') for node in doc.get('nodes', [])
                     if 'mesh' in node and any(p.get('targets') for p in doc.get('meshes', [])[node['mesh']].get('primitives', []))}
    for obj in scene.objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        if obj.data.shape_keys is not None and obj.name not in morph_objects:
            findings.append(dict(object=obj.name, feature='shape keys', replacement='evaluated source-frame mesh; no morph targets were emitted for this object'))
        if not apply_modifiers:
            for modifier in obj.modifiers:
                if modifier.show_render and modifier.type != 'ARMATURE':
                    findings.append(dict(object=obj.name, feature=modifier.name,
                                         replacement='omit non-armature modifier to retain native shape keys'))
    return dict(layers=[dict(clip=dict(target=asset_name, index=i)) for i in range(len(clips))],
                channels=len(channels), paths=sorted({c['target']['path'] for c in channels}),
                skins=len(doc.get('skins', [])), findings=findings)


def camera_focus(obj, deps):
    """Return source focus distance and whether it is outside thin-lens support."""
    data = obj.data
    distance = data.dof.focus_distance
    if data.dof.focus_object is not None:
        focus = data.dof.focus_object.evaluated_get(deps).matrix_world.translation
        axis = obj.matrix_world.to_3x3().normalized().col[2]
        distance = abs((focus-obj.matrix_world.translation).dot(axis))
    # Match the renderer's f32 millimetre conversion and native input precision.
    f32 = lambda value: struct.unpack('<f', struct.pack('<f', value))[0]
    focal_length = f32(f32(data.lens) * f32(.001))
    return float(distance), f32(distance) <= focal_length


def sample_controls(scene, bindings, *, frames=None):
    """bindings maps explicit vork node names to source camera/light objects.

    Rigid mesh/armature transforms and morph weights belong to export_clip's
    imported hierarchy. These explicit records use world-space evaluated poses.
    Values are sampled at source frames and linearly interpolated between them.
    """
    frames = list(range(scene.frame_start, scene.frame_end+1)) if frames is None else list(frames)
    if not frames or any(b <= a for a,b in zip(frames,frames[1:])):
        raise ValueError('animation frames must be a nonempty increasing sequence')
    fps = scene.render.fps / scene.render.fps_base
    times = [(frame-frames[0])/fps for frame in frames]
    original = scene.frame_current, scene.frame_subframe
    values, previous, findings = {}, {}, []
    disabled_dof = set()
    camera_names = {obj: name for name,obj in bindings.items()}
    cameras = []
    initial = {}
    object_kinds, snapshot_lights = {}, set()
    def put(name, path, value):
        values.setdefault((name, tuple(path)), []).append(value)
    try:
        with gltf_render_settings(scene):
            for frame in frames:
                scene.frame_set(frame)
                deps = bpy.context.evaluated_depsgraph_get()
                cameras.append(camera_names.get(scene.camera))
                for name, source in bindings.items():
                    obj = source.evaluated_get(deps)
                    object_kinds.setdefault(name,obj.type)
                    if object_kinds[name] != obj.type:
                        raise ValueError(f'{source.name}: camera/light object-kind animation is not represented')
                    world = obj.matrix_world
                    converted = _BASIS @ world
                    translation = converted.translation
                    rotation = pose_rotation(converted, area=obj.type == 'LIGHT' and initial.get(name,(obj.data.type,))[0] == 'AREA')
                    if name in previous and rotation.dot(previous[name]) < 0:
                        rotation.negate()
                    previous[name] = rotation.copy()
                    put(name,['translation'],list(translation))
                    put(name,['rotation'],[rotation.x,rotation.y,rotation.z,rotation.w])
                    data = obj.data
                    if obj.type == 'CAMERA':
                        configuration = (data.type, data.sensor_fit, data.dof.use_dof)
                        if name not in initial:
                            initial[name] = configuration
                        elif configuration != initial[name]:
                            finding = dict(object=source.name, feature='projection/sensor_fit/DOF enable animation', replacement='keep exported initial configuration; only keyable numeric controls are sampled')
                            if finding not in findings:
                                findings.append(finding)
                        put(name,['shift'],[float(data.shift_x),float(data.shift_y)])
                        fields = ['clip_start','clip_end']
                        fields += ['ortho_scale'] if initial[name][0] == 'ORTHO' else ['lens','sensor_width','sensor_height']
                        sampled = {field: float(getattr(data,field)) for field in fields}
                        if 'sensor_height' in sampled:
                            # The exported (initial) fit decides: Blender AUTO in portrait spans the height with sensor_width.
                            fit = camera_fit(SimpleNamespace(sensor_fit=initial[name][1], sensor_width=sampled['sensor_width']),
                                             scene.render.resolution_x, scene.render.resolution_y)
                            sampled.update({k: float(v) for k, v in fit.items() if k == 'sensor_height'})
                        for field, value in sampled.items():
                            put(name,[field],value)
                        if initial[name][2] and initial[name][0] == 'PERSP':
                            for field in ('aperture_rotation','aperture_ratio','aperture_fstop'):
                                put(name,['dof',field],float(getattr(data.dof,field)))
                            distance, substituted = camera_focus(obj, deps)
                            if substituted:
                                disabled_dof.add(name)
                                finding = dict(object=source.name, feature='DOF focus plane at or inside focal length', replacement='omit DOF for the entire sampled clip')
                                if finding not in findings:
                                    findings.append(finding)
                            put(name,['dof','focus_distance'],distance)
                    elif obj.type == 'LIGHT':
                        configuration=(data.type,data.shape if data.type=='AREA' else None,data.normalize,data.use_shadow,data.use_temperature)
                        if name not in initial:
                            initial[name]=configuration
                        elif configuration != initial[name]:
                            # A temperature track needs the exported light to carry a temperature.
                            incompatible=data.type!=initial[name][0] or data.normalize!=initial[name][2] or data.use_temperature!=initial[name][4]
                            if incompatible:
                                snapshot_lights.add(name)
                            finding=dict(object=source.name,feature='light type/shape/normalization/shadow animation',replacement='snapshot numeric light data for this clip; sample transforms' if incompatible else 'keep exported initial configuration; sample supported numeric controls')
                            if finding not in findings:
                                findings.append(finding)
                            if incompatible:
                                continue
                        put(name,['color'],light_color(data))
                        if data.use_temperature:
                            put(name,['temperature'],light_temperature(data))
                        put(name,['energy'],light_energy(data))
                        fields = {'SUN':('angle',), 'POINT':('radius',),
                                  'SPOT':('radius','spot_size','spot_blend'),
                                  'AREA':('size',)}.get(data.type,())
                        for field in fields:
                            value = float(getattr(data,'shadow_soft_size' if field == 'radius' else field))
                            if data.type == 'AREA' and field == 'size':
                                value *= world.to_3x3().col[0].length
                            put(name,[field],value)
                        if data.type == 'AREA':
                            put(name,['size_y'],float(data.size_y if initial[name][1] in ('RECTANGLE','ELLIPSE') else data.size)*world.to_3x3().col[1].length)
                    else:
                        raise ValueError(f'{source.name}: explicit animation binding must be a camera or light')
    finally:
        scene.frame_set(original[0], subframe=original[1])
    def track_path(name, path):
        if bindings[name].type == 'CAMERA' and path[0] not in ('translation','rotation'):
            return ['materials',name+'-optics','inputs',path[-1]]
        return ['nodes',name,*path]
    tracks = [dict(path=track_path(name,path),keys=[[t,v] for t,v in zip(times,rows)],interpolation='LINEAR')
              for (name,path),rows in values.items() if not (name in disabled_dof and path[0] == 'dof')
              and not (name in snapshot_lights and path not in (('translation',),('rotation',)))
              and any(row != rows[0] for row in rows[1:])]
    if any(camera != cameras[0] for camera in cameras[1:]):
        if all(camera is not None for camera in cameras):
            tracks.append(dict(path=['render','camera'],keys=[[t,c] for t,c in zip(times,cameras)],interpolation='STEP'))
        else:
            findings.append(dict(feature='camera cuts',replacement='keep initial camera; not all source cameras have explicit bindings'))
    return dict(layer=dict(tracks=tracks), source_frames=frames, findings=findings, disabled_dof=sorted(disabled_dof))
