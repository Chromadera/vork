# Generated from scene.schema.json; do not edit.
from __future__ import annotations
from typing import Any, Literal, Union, TypedDict, Required

AimTarget = Union[str, list[float]]
AovType = Literal['value', 'color']
Background = Union[list[float], 'BackgroundMode']
BackgroundMode = Literal['environment', 'transparent']
Blend = Literal['replace', 'add']
Buffer = Union['InlineBuffer', 'FileBuffer', 'DenseBuffer']
BufferType = Literal['f32', 'i32', 'u32', 'vec2', 'vec3', 'vec4', 'uvec4', 'mat4']
Channel = Literal['r', 'g', 'b', 'a', 'rgb']
Clock = Union[float, str]
ColorSpace = Literal['sRGB', 'Linear Rec.709', 'Non-Color']
CompositeEntry = Union['NameOr', 'CompositeNode']
CompositeResolution = Union[list[int], float]
ComputeDispatch = Union[int, list[int]]
ComputeSchedule = Union[Literal['frame', 'install'], Literal['simulation']]
Factor = Union[float, list[float]]
InlineDataSchema = Union[list[float], list[list[float]]]
Input = Union[float, list[float], bool, 'TextureBinding', 'AttributeBinding', 'GridBinding', 'AovBinding', 'ChannelBinding', 'NodeBinding', str]
Interpolation = Literal['STEP', 'LINEAR', 'CUBICSPLINE']
KernelChannel = Literal['color', 'depth', 'view_normal', 'object_index', 'motion', 'shadow_catcher']
Keys = Union[list[list[Any]], 'Keys1']
Layer = Union['TrackLayer', 'ClipLayer']
LightShape = Literal['sun', 'point', 'spot', 'rectangle', 'ellipse', 'environment']
MagFilter = Literal['nearest', 'linear']
MinFilter = Literal['nearest', 'linear', 'nearest_mipmap_nearest', 'linear_mipmap_nearest', 'nearest_mipmap_linear', 'linear_mipmap_linear']
MotionBlurPosition = Literal['start', 'center', 'end']
NameOr = Union[str, 'Material']
NameOr2 = Union[str, 'Program']
NameOr3 = Union[str, 'Buffer']
NameOr4 = Union[str, 'Image']
NameOr5 = Union[str, 'Geometry']
Node = Union['TransformNode', 'MeshNode', 'AssetNode', 'CameraNode', 'LightNode']
Opacity = Literal['coverage', 'transmittance']
Profile = 'ProfileFile'
Program = Union['FileProgram', 'SourceProgram']
Rotation = Union[list[float], 'Rotation1']
SubdivisionType = Literal['catmull_clark', 'simple']
Time = Union[float, str]
Topology = Union[Literal['triangles', 'points', 'curves'], Literal['implicit']]
Wrap = Literal['repeat', 'clamp_to_edge', 'mirrored_repeat']
Aov = TypedDict('Aov', {'name': Required[str], 'type': Required['AovType']}, total=False)
AovBinding = TypedDict('AovBinding', {'aov': Required[str]}, total=False)
AssetNodeOverrides = TypedDict('AssetNodeOverrides', {'light_link_tags': Union[list[str], None], 'pass_index': Union[int, None], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
AttributeBinding = TypedDict('AttributeBinding', {'attribute': Required[str], 'channel': Union['Channel', None], 'factor': Union['Factor', None]}, total=False)
InlineBuffer = TypedDict('InlineBuffer', {'count': Union[int, None], 'data': Required[Union['InlineDataSchema']], 'dims': None, 'file': None, 'grid': Union[str, None], 'offset': int, 'type': Required['BufferType']}, total=False)
FileBuffer = TypedDict('FileBuffer', {'count': Union[int, None], 'data': None, 'dims': None, 'file': Required[Union[str]], 'grid': Union[str, None], 'offset': int, 'type': Required['BufferType']}, total=False)
DenseBuffer = TypedDict('DenseBuffer', {'count': None, 'data': Union['InlineDataSchema', None], 'dims': Required[list[int]], 'file': None, 'grid': None, 'offset': int, 'type': Required[Literal['f32', 'vec3']]}, total=False)
ChannelBinding = TypedDict('ChannelBinding', {'channel': Required['KernelChannel']}, total=False)
Clip = TypedDict('Clip', {'file': Union[str, None], 'index': Union[int, None], 'name': Union[str, None], 'target': Required[str]}, total=False)
CompositeNode = TypedDict('CompositeNode', {'material': Required['NameOr'], 'name': Union[str, None], 'resolution': Union['CompositeResolution', None]}, total=False)
EdgeCrease = TypedDict('EdgeCrease', {'crease': Required[float], 'vertices': Required[list[int]]}, total=False)
Geometry = TypedDict('Geometry', {'attributes': Required[dict[str, 'NameOr3']], 'count': Union['NameOr3', None], 'crease_edge': list['EdgeCrease'], 'curve_vertex_counts': Union['NameOr3', None], 'face_vertex_counts': Union[list[int], None], 'ids': Union['NameOr3', None], 'indices': Union['NameOr3', None], 'material_ranges': list['MaterialRange'], 'morphs': dict[str, 'Morph'], 'subdivision': Union['SubdivisionSurface', None], 'topology': 'Topology', 'widths': Union['NameOr3', None]}, total=False)
GridBinding = TypedDict('GridBinding', {'grid': Required['NameOr3']}, total=False)
Image = TypedDict('Image', {'color_space': Union['ColorSpace', None], 'file': Union[str, None], 'mag_filter': 'MagFilter', 'min_filter': 'MinFilter', 'render': Union['ImageRender', None], 'time': Union['Clock', None], 'wrap_s': 'Wrap', 'wrap_t': 'Wrap'}, total=False)
ImageRender = TypedDict('ImageRender', {'background': 'Background', 'camera': Union[str, None], 'compositor': list['CompositeEntry'], 'resolution': list[int], 'root': Union[str, None]}, total=False)
Instances = TypedDict('Instances', {'attributes': Required[dict[str, 'NameOr3']], 'count': Union['NameOr3', None]}, total=False)
Keys1 = TypedDict('Keys1', {'times': Required['NameOr3'], 'values': Required['NameOr3']}, total=False)
TrackLayer = TypedDict('TrackLayer', {'blend': 'Blend', 'clip': None, 'loop': bool, 'name': Union[str, None], 'offset': float, 'speed': float, 'tracks': Required[Union[list['Track']]], 'weight': float}, total=False)
ClipLayer = TypedDict('ClipLayer', {'blend': 'Blend', 'clip': Required[Union['Clip']], 'loop': bool, 'name': Union[str, None], 'offset': float, 'speed': float, 'tracks': None, 'weight': float}, total=False)
Material = TypedDict('Material', {'double_sided': bool, 'inputs': dict[str, 'Input'], 'opacity': Union['Opacity', None], 'program': 'NameOr2'}, total=False)
MaterialRange = TypedDict('MaterialRange', {'count': Required[int], 'material_index': Required[int], 'start': Required[int]}, total=False)
Morph = TypedDict('Morph', {'NORMAL': Union['NameOr3', None], 'POSITION': Required['NameOr3']}, total=False)
MotionBlur = TypedDict('MotionBlur', {'position': 'MotionBlurPosition', 'shutter': float}, total=False)
TransformNode = TypedDict('TransformNode', {'children': list['Node'], 'is_holdout': bool, 'is_shadow_catcher': bool, 'light_link_tags': list[str], 'name': str, 'pass_index': Union[int, None], 'rotation': 'Rotation', 'scale': list[float], 'target': Union['AimTarget', None], 'translation': list[float], 'type': Literal['node'], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
MeshNode = TypedDict('MeshNode', {'children': list['Node'], 'geometry': Required['NameOr5'], 'instances': Union['Instances', None], 'is_holdout': bool, 'is_shadow_catcher': bool, 'light_link_tags': list[str], 'materials': list['NameOr'], 'morph_weights': dict[str, float], 'name': str, 'pass_index': Union[int, None], 'rotation': 'Rotation', 'scale': list[float], 'skin': Union['Skin', None], 'target': Union['AimTarget', None], 'translation': list[float], 'type': Required[Literal['mesh']], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
AssetNode = TypedDict('AssetNode', {'children': list['Node'], 'file': Required[str], 'is_holdout': bool, 'is_shadow_catcher': bool, 'light_link_tags': list[str], 'materials': dict[str, 'NameOr'], 'name': str, 'nodes': dict[str, 'AssetNodeOverrides'], 'pass_index': Union[int, None], 'rotation': 'Rotation', 'scale': list[float], 'target': Union['AimTarget', None], 'translation': list[float], 'type': Required[Literal['asset']], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
CameraNode = TypedDict('CameraNode', {'children': list['Node'], 'focus_object': Union[str, None], 'is_holdout': bool, 'is_shadow_catcher': bool, 'light_link_tags': list[str], 'material': Union['NameOr', None], 'name': str, 'pass_index': Union[int, None], 'rotation': 'Rotation', 'scale': list[float], 'target': Union['AimTarget', None], 'translation': list[float], 'type': Required[Literal['camera']], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
LightNode = TypedDict('LightNode', {'angle': float, 'children': list['Node'], 'color': Union[list[float], None], 'energy': Union[float, None], 'illuminates': Union[list[str], None], 'image': Union['NameOr4', None], 'is_holdout': bool, 'is_shadow_catcher': bool, 'light_link_tags': list[str], 'material': Union['NameOr', None], 'name': str, 'pass_index': Union[int, None], 'radius': float, 'rotation': 'Rotation', 'scale': list[float], 'shadowed_by': Union[list[str], None], 'shape': Required['LightShape'], 'size': float, 'size_y': Union[float, None], 'spot_blend': float, 'spot_size': float, 'target': Union['AimTarget', None], 'temperature': Union[float, None], 'translation': list[float], 'type': Required[Literal['light']], 'visible': bool, 'visible_camera': bool, 'visible_diffuse': bool, 'visible_glossy': bool, 'visible_shadow': bool, 'visible_transmission': bool}, total=False)
NodeBinding = TypedDict('NodeBinding', {'node': Required[str]}, total=False)
ProfileFile = TypedDict('ProfileFile', {'description': str, 'level': Required[int], 'settings': 'Settings', 'version': Required[int]}, total=False)
FileProgram = TypedDict('FileProgram', {'bindings': dict[str, 'NameOr3'], 'dispatch': Union['ComputeDispatch', None], 'file': Required[Union[str]], 'schedule': Union['ComputeSchedule', None], 'source': None, 'source_origin': None}, total=False)
SourceProgram = TypedDict('SourceProgram', {'bindings': dict[str, 'NameOr3'], 'dispatch': Union['ComputeDispatch', None], 'file': None, 'schedule': Union['ComputeSchedule', None], 'source': Required[Union[str]], 'source_origin': Union[str, None]}, total=False)
Propagation = TypedDict('Propagation', {'bounds': Required[list[list[float]]], 'program': Required[str]}, total=False)
Render = TypedDict('Render', {'aovs': list['Aov'], 'background': 'Background', 'camera': Union[str, None], 'compositor': list['CompositeEntry'], 'display': 'NameOr', 'duration': 'Clock', 'fps': 'Time', 'motion_blur': Union['MotionBlur', None], 'resolution': list[int], 'root': Union[str, None]}, total=False)
Rotation1 = TypedDict('Rotation1', {'euler': Required[list[float]]}, total=False)
Settings = TypedDict('Settings', {'denoise': bool, 'samples': int, 'seed': int}, total=False)
Simulation = TypedDict('Simulation', {'dt': Required['Clock'], 'init': Required[list[str]], 'seed': Required[int], 'state': Required[list[str]], 'steps': Required[list[str]], 't0': Required['Clock']}, total=False)
Skin = TypedDict('Skin', {'inverse_bind_matrices': Required['NameOr3'], 'joints': Required[list[str]]}, total=False)
SubdivisionSurface = TypedDict('SubdivisionSurface', {'render_levels': int, 'subdivision_type': 'SubdivisionType', 'use_limit_surface': bool}, total=False)
TextureBinding = TypedDict('TextureBinding', {'image': Required['NameOr4']}, total=False)
Track = TypedDict('Track', {'interpolation': Union['Interpolation', None], 'keys': Required['Keys'], 'path': Required[list[str]]}, total=False)
Scene = TypedDict('Scene', {'$schema': Union[str, None], 'animation': list['Layer'], 'buffers': dict[str, 'Buffer'], 'geometry': dict[str, 'Geometry'], 'images': dict[str, 'Image'], 'materials': dict[str, 'Material'], 'nodes': Required[list['Node']], 'programs': dict[str, 'Program'], 'propagation': Union['Propagation', None], 'render': 'Render', 'simulation': Union['Simulation', None], 'version': Required[Literal[2]]}, total=False)
