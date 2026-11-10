"""Snapshot a directional World through Cycles into a scene-linear lat-long map.

This is a bridge bake, never a fallback to an unrelated Background node. Reject
observer/ray-dependent worlds: a directional environment cannot represent them.
"""
import math


def validate(socket, seen=None):
    from .world import UnsupportedWorld
    seen = set() if seen is None else seen
    if not socket.is_linked: return
    link = socket.links[0]; node = link.from_node
    key = node.as_pointer()
    if key in seen: return
    seen.add(key)
    allowed = {'OUTPUT_WORLD', 'BACKGROUND', 'MIX_SHADER', 'ADD_SHADER', 'TEX_SKY',
               'TEX_ENVIRONMENT', 'TEX_IMAGE', 'MAPPING', 'TEX_COORD', 'RGB', 'VALUE',
               'BLACKBODY', 'MATH', 'MAP_RANGE', 'MIX', 'MIX_RGB', 'REROUTE'}
    if node.type not in allowed:
        raise UnsupportedWorld(f'World {node.name!r}.{link.from_socket.name}: {node.type} cannot be baked as a directional environment')
    if node.type == 'TEX_COORD':
        if link.from_socket.name not in ('Generated', 'Normal', 'Object') or node.object is not None:
            raise UnsupportedWorld(f'World {node.name!r}.{link.from_socket.name}: observer-dependent coordinates cannot be baked as a directional environment')
    if node.type == 'TEX_IMAGE' and node.image is None:
        raise UnsupportedWorld(f'World {node.name!r}: missing image')
    for inp in node.inputs:
        if inp.enabled: validate(inp, seen)


def bake(world, directory, *, width=2048):
    """Render only World radiance; source scene, World and render settings stay intact."""
    import bpy
    from pathlib import Path
    from .world import UnsupportedWorld, WorldImport, medium
    output = world.node_tree.get_output_node('CYCLES')
    if output is None: raise UnsupportedWorld('World: no active Cycles output')
    validate(output.inputs['Surface'])
    materials = medium(output.inputs['Volume'])
    scene = bpy.data.scenes.new('vork directional World bake')
    copied_world = world.copy()
    camera_data = bpy.data.cameras.new('vork World bake camera')
    camera = bpy.data.objects.new('vork World bake camera', camera_data)
    try:
        scene.collection.objects.link(camera); scene.camera = camera
        scene.world = copied_world
        copied_output = copied_world.node_tree.get_output_node('CYCLES')
        for link in list(copied_output.inputs['Volume'].links): copied_world.node_tree.links.remove(link)
        camera_data.type = 'PANO'; camera_data.panorama_type = 'EQUIRECTANGULAR'
        # Preserve the source environment texture's lat-long orientation. The
        # directional RGB oracle checks this Z-up to Y-up boundary end to end.
        camera.rotation_euler = (math.pi/2, 0, -math.pi/2)
        scene.render.engine = 'CYCLES'
        source = bpy.context.scene
        scene.cycles.device = source.cycles.device
        scene.cycles.samples = 1; scene.cycles.use_adaptive_sampling = False
        scene.cycles.use_denoising = False
        scene.render.resolution_x = width; scene.render.resolution_y = width//2
        scene.render.resolution_percentage = 100
        scene.render.film_transparent = False
        scene.render.filter_size = .01
        scene.render.use_compositing = False; scene.render.use_sequencer = False
        scene.render.image_settings.file_format = 'OPEN_EXR'
        scene.render.image_settings.color_depth = '32'
        scene.render.filepath = str(Path(directory).resolve()/'world-baked.exr')
        bpy.ops.render.render(scene=scene.name, write_still=True)
    finally:
        bpy.data.objects.remove(camera, do_unlink=True)
        bpy.data.cameras.remove(camera_data)
        bpy.data.scenes.remove(scene)
        bpy.data.worlds.remove(copied_world)
    node = dict(type='light', name='environment', shape='environment', color=[1,1,1], energy=1,
                material={'program':'std/environment_image', 'inputs':{'image':{'image':{'file':'world-baked.exr','color_space':'Linear Rec.709'}}}})
    if materials: node['medium'] = 'world_medium'
    return WorldImport(None, node, 'environment', materials)
