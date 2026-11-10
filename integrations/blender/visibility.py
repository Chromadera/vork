"""Ray flags for the exact exported glTF node identities, including instances."""
import json
from pathlib import Path
import struct

FLAGS = ('visible_camera', 'visible_shadow', 'visible_diffuse', 'visible_glossy', 'visible_transmission')
METADATA = '_vork_ray_visibility'


def flags(obj):
    return {name: bool(getattr(obj, name)) for name in FLAGS}


def asset_overrides(path, on_substitution):
    result = {}
    data = Path(path).read_bytes()
    document = json.loads(data[20:20+struct.unpack_from('<I', data, 12)[0]]) if data[:4] == b'glTF' else json.loads(data)
    for index, node in enumerate(document.get('nodes', [])):
        source = node.get('extras', {}).get(METADATA)
        if source is None: continue
        record = json.loads(source)
        visibility = record['flags']
        if not all(visibility.values()): result[f'#{index}'] = visibility
        if not record['visible_volume_scatter']:
            on_substitution(record['object'], 'visible_volume_scatter=false has no independent vork equivalent',
                            'retain vork volume-scatter visibility; map the other five ray flags independently')
    return result
