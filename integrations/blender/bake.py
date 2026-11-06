# SPDX-License-Identifier: GPL-3.0-or-later
"""CPU Cycles emission bakes of view-independent scalar/colour source sockets.

A bake belongs to one evaluated object, UV layout and source frame. It cannot be
shared as a material-wide replacement across unrelated meshes or animation poses.
"""
from pathlib import Path
import bpy
from .materials import UnsupportedMaterial


def view_independent(socket):
    seen = set()
    def visit(socket):
        node = socket.node
        key = node.as_pointer(), socket.identifier
        if key in seen:
            return
        seen.add(key)
        if node.type in ('LIGHT_PATH', 'LAYER_WEIGHT', 'FRESNEL', 'SHADER_TO_RGB', 'BSDF_HAIR', 'BSDF_HAIR_PRINCIPLED', 'OBJECT_INFO'):
            raise UnsupportedMaterial(f'{node.name}: view/ray-dependent values cannot be texture baked')
        if node.type in ('TEX_IMAGE','TEX_ENVIRONMENT'):
            image=node.image
            if image is None:
                raise UnsupportedMaterial(f'{node.name}: source image is missing')
            if image.source not in ('FILE', 'GENERATED'):
                raise UnsupportedMaterial(f'{node.name}: {image.source} image evaluation is not qualified for baking')
            if image.is_dirty or image.source == 'GENERATED':
                if not image.has_data or not all(image.size):
                    raise UnsupportedMaterial(f'{node.name}: source image pixels are unavailable')
            elif image.source=='FILE' and not image.packed_file:
                filename=Path(bpy.path.abspath(image.filepath,library=image.library))
                if not filename.is_file():
                    raise UnsupportedMaterial(f'{node.name}: source image file is unavailable: {filename}')
        if node.type == 'NEW_GEOMETRY' and socket.name != 'Position':
            raise UnsupportedMaterial(f'{node.name}.{socket.name}: geometry response is not qualified for baking')
        if node.type in ('TEX_COORD','UVMAP') and getattr(node, 'from_instancer', False):
            raise UnsupportedMaterial(f'{node.name}: instancer coordinates are unavailable in the bake mesh')
        if node.type == 'TEX_COORD' and socket.name not in ('UV', 'Object'):
            raise UnsupportedMaterial(f'{node.name}.{socket.name}: view-dependent coordinates cannot be texture baked')
        if node.type.startswith('TEX_') and node.type not in ('TEX_COORD', 'TEX_IMAGE'):
            vector = node.inputs.get('Vector')
            if vector is not None and not vector.is_unavailable and not vector.is_linked:
                raise UnsupportedMaterial(f'{node.name}: implicit rest-Generated coordinates are unavailable in the bake mesh')
        if node.type == 'ATTRIBUTE' and node.attribute_type != 'GEOMETRY':
            raise UnsupportedMaterial(f'{node.name}: object/instancer attributes are unavailable in the bake mesh')
        if node.type == 'GROUP':
            output = next((n for n in node.node_tree.nodes if n.type == 'GROUP_OUTPUT' and n.is_active_output), None)
            if output is None:
                raise UnsupportedMaterial(f'{node.name}: missing group output')
            target = next((s for s in output.inputs if s.identifier == socket.identifier), None)
            if target is None:
                raise UnsupportedMaterial(f'{node.name}: missing group output socket')
            for link in target.links:
                visit(link.from_socket)
        for input in node.inputs:
            if not getattr(input, 'is_unavailable', False):
                for link in input.links:
                    visit(link.from_socket)
    visit(socket)


def bake_value(material, socket, obj, path, resolution):
    """Write a scene-linear float EXR and restore all source state on failure too.

    Resolution is an explicit caller decision. The mesh must already carry an
    active render UV; no automatic unwrap or fabricated source coordinates.
    """
    if socket.type not in ('VALUE', 'VECTOR', 'RGBA') or socket.node.id_data != material.node_tree:
        raise UnsupportedMaterial('bake needs a numeric output on the material root tree')
    if obj is None or obj.type != 'MESH':
        raise UnsupportedMaterial('bake needs its source mesh object')
    if not isinstance(resolution, int) or resolution <= 0:
        raise ValueError('bake resolution must be a positive integer')
    view_independent(socket)
    scene = bpy.context.scene
    deps = bpy.context.evaluated_depsgraph_get()
    copy = image = temporary = None
    selected = list(bpy.context.selected_objects)
    active = bpy.context.view_layer.objects.active
    engine, device, samples = scene.render.engine, scene.cycles.device, scene.cycles.samples
    path = Path(path).resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    mesh = bpy.data.meshes.new_from_object(obj.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
    try:
        # A shared source material can occupy only some polygons. Do not bake
        # a different slot over its UV islands or silently replace that slot.
        slots = {i for i, m in enumerate(mesh.materials) if m == material or getattr(m, 'original', None) == material}
        if not slots:
            raise UnsupportedMaterial(f'{obj.name}: source material is not assigned to this mesh')
        import bmesh
        bm = bmesh.new()
        try:
            bm.from_mesh(mesh)
            bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.material_index not in slots], context='FACES')
            bm.to_mesh(mesh)
        finally:
            bm.free()
        if not mesh.polygons:
            raise UnsupportedMaterial(f'{obj.name}: no polygons use the baked material')
        uv = next((layer for layer in mesh.uv_layers if layer.active_render), None)
        if uv is None:
            raise UnsupportedMaterial(f'{obj.name}: bake requires an active render UV')
        from .bake_uv import qualify_mesh
        qualify_mesh(mesh,uv)
        mesh.uv_layers.active = uv
        copy = material.copy()
        mesh.materials.clear()
        mesh.materials.append(copy)
        for polygon in mesh.polygons:
            polygon.material_index = 0
        temporary = bpy.data.objects.new('__vork_value_bake__', mesh)
        scene.collection.objects.link(temporary)
        temporary.matrix_world = obj.matrix_world.copy()
        nodes, links = copy.node_tree.nodes, copy.node_tree.links
        source = next(s for s in nodes[socket.node.name].outputs if s.identifier == socket.identifier)
        for node in list(nodes):
            if node.type == 'OUTPUT_MATERIAL':
                nodes.remove(node)
        emission = nodes.new('ShaderNodeEmission')
        links.new(source, emission.inputs['Color'])
        emission.inputs['Strength'].default_value = 1
        output = nodes.new('ShaderNodeOutputMaterial')
        links.new(emission.outputs[0], output.inputs['Surface'])
        image = bpy.data.images.new('__vork_value_bake__', width=resolution, height=resolution,
                                    alpha=False, float_buffer=True)
        image.colorspace_settings.name = 'Linear Rec.709'
        target = nodes.new('ShaderNodeTexImage')
        target.image = image
        nodes.active = target
        for o in selected:
            o.select_set(False)
        temporary.select_set(True)
        bpy.context.view_layer.objects.active = temporary
        scene.render.engine = 'CYCLES'
        scene.cycles.device = 'CPU'
        scene.cycles.samples = 1  # Emission has no light-transport estimator.
        result = bpy.ops.object.bake(type='EMIT', use_clear=True, margin=scene.render.bake.margin,
                                     use_selected_to_active=False, target='IMAGE_TEXTURES', uv_layer=uv.name)
        if 'FINISHED' not in result:
            raise RuntimeError(f'value bake did not finish: {result}')
        image.file_format = 'OPEN_EXR'
        image.filepath_raw = str(path)
        image.save()
        return dict(file=str(path), color_space='Linear Rec.709',
                    object=obj.name, frame=float(scene.frame_current_final), resolution=resolution,
                    uv_layer=uv.name, margin=scene.render.bake.margin)
    finally:
        scene.render.engine = engine
        scene.cycles.device, scene.cycles.samples = device, samples
        if temporary is not None:
            bpy.data.objects.remove(temporary, do_unlink=True)
        bpy.data.meshes.remove(mesh)
        if copy is not None:
            bpy.data.materials.remove(copy)
        if image is not None:
            bpy.data.images.remove(image)
        for o in selected:
            o.select_set(True)
        bpy.context.view_layer.objects.active = active
