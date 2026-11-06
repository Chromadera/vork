"""Generate Python input types from the Rust-generated JSON Schema."""
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parent
schema = json.loads((root / '../ts/scene.schema.json').read_text())
definitions = schema.get('$defs', schema.get('definitions', {}))
lines = ['# Generated from scene.schema.json; do not edit.', 'from __future__ import annotations', 'from typing import Any, Literal, Union, TypedDict, Required', '']
pending = []

def type_of(s, name):
    if not isinstance(s, dict):
        return 'Any'
    if '$ref' in s:
        return repr(s['$ref'].split('/')[-1])
    if 'const' in s:
        return f'Literal[{s["const"]!r}]'
    if 'enum' in s:
        return 'Literal[' + ', '.join(repr(v) for v in s['enum']) + ']'
    for key in ('anyOf', 'oneOf'):
        if key in s:
            return 'Union[' + ', '.join(type_of(v, name + str(i)) for i, v in enumerate(s[key])) + ']'
    name = s.get('title', name)
    t = s.get('type')
    if isinstance(t, list):
        return 'Union[' + ', '.join(type_of({**s, 'type': v}, name) for v in t) + ']'
    if t == 'object' or 'properties' in s:
        if 'properties' in s:
            pending.append((name, s))
            return repr(name)
        return 'dict[str, ' + type_of(s.get('additionalProperties') if isinstance(s.get('additionalProperties'), dict) else {}, name+'Value') + ']'
    if t == 'array':
        items = s.get('items', {})
        if isinstance(items, list):
            return 'tuple[' + ', '.join(type_of(v, name+'Item'+str(i)) for i,v in enumerate(items)) + ']'
        return 'list[' + type_of(items, name+'Item') + ']'
    return {'string':'str','number':'float','integer':'int','boolean':'bool','null':'None'}.get(t, 'Any')

for name, value in [*definitions.items(), ('Scene', schema)]:
    result = type_of(value, name)
    if result != repr(name):
        lines.append(f'{name} = {result}')
seen = set()
while pending:
    name, value = pending.pop(0)
    if name in seen:
        continue
    seen.add(name)
    fields = []
    for key, field in value['properties'].items():
        annotation = type_of(field, name + key.title().replace('_', ''))
        if key in value.get('required', []):
            annotation = f'Required[{annotation}]'
        fields.append(f'{key!r}: {annotation}')
    lines.append(f'{name} = TypedDict({name!r}, {{{", ".join(fields)}}}, total=False)')
text = '\n'.join(lines) + '\n'
target = root / 'vork/types.py'
if '--check' in sys.argv:
    assert target.read_text() == text, 'stale Python types; run scripts/generate-docs'
else:
    target.write_text(text)
