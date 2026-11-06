# SPDX-License-Identifier: GPL-3.0-or-later
"""Render occurrence selection and temporary glTF render evaluation settings."""
from contextlib import contextmanager


def direct_render_objects(layer_collection):
    result = set()

    def visit(layer):
        if layer.exclude or layer.collection.hide_render:
            return
        result.update(obj for obj in layer.collection.objects if not obj.hide_render)
        for child in layer.children:
            visit(child)

    visit(layer_collection)
    return result


@contextmanager
def render_selection(scene):
    import bpy
    layer = bpy.context.view_layer
    selected = [(obj, obj.select_get()) for obj in layer.objects]
    active = layer.objects.active
    visible = direct_render_objects(layer.layer_collection)
    try:
        # glTF inherits selection for collection contents, not ordinary parent/child objects.
        for obj, _ in selected:
            obj.select_set(obj in visible and (obj.type != 'CAMERA' or obj == scene.camera))
        yield
    finally:
        for obj, value in selected:
            obj.select_set(value)
        layer.objects.active = active


@contextmanager
def gltf_render_settings(scene):
    import bpy
    restored = []

    def set_value(owner, name, value):
        before = getattr(owner, name)
        if before != value:
            restored.append((owner, name, before))
            setattr(owner, name, value)

    try:
        # Include off-scene collection templates: visible instancers still need their data.
        for obj in bpy.data.objects:
            set_value(obj, 'hide_viewport', False)
            for modifier in obj.modifiers:
                set_value(modifier, 'show_viewport', modifier.show_render)
                if modifier.type in {'SUBSURF', 'MULTIRES'}:
                    set_value(modifier, 'levels', modifier.render_levels)
        # glTF already converts evaluated curves to meshes; its depsgraph is viewport mode.
        for curve in bpy.data.curves:
            if curve.render_resolution_u:
                for spline in curve.splines:
                    set_value(spline, 'resolution_u', curve.render_resolution_u)
            if curve.render_resolution_v:
                for spline in curve.splines:
                    set_value(spline, 'resolution_v', curve.render_resolution_v)
        for collection in bpy.data.collections:
            set_value(collection, 'hide_viewport', False)
        if scene.render.use_simplify:
            set_value(scene.render, 'simplify_subdivision', scene.render.simplify_subdivision_render)
        bpy.context.view_layer.update()
        yield
    finally:
        for owner, name, value in reversed(restored):
            setattr(owner, name, value)
        bpy.context.view_layer.update()
