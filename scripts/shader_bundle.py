"""Release assets from the renderer's portable, content-addressed shader entries."""
import hashlib
import json
from pathlib import Path
import re
import shutil

ENTRY = re.compile(r'[0-9a-f]{64}\.(?:slang-variant-v1|slang-program-v2|slang-resolve-v2|aov-resolve-v1)\.entry')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def export(binary, caches, output):
    # Preparation needs BLAKE3 to verify the renderer's existing cache envelope.
    # Packaging remains stdlib-only: its SHA-256 manifest binds these verified bytes.
    try:
        from blake3 import blake3
    except ImportError as error:
        raise RuntimeError('shader preparation requires blake3; run with uv run --with blake3==1.0.8 python scripts/prepare-shaders') from error
    entries = {}
    corrupt = {}
    for cache in caches:
        for entry in sorted(Path(cache).glob('*.entry')):
            if not ENTRY.fullmatch(entry.name):
                continue
            data = entry.read_bytes()
            if len(data) <= 32 or data[:32] != blake3(b'vork-cache-entry-v2\0' + entry.name.encode() + b'\0' + data[32:]).digest():
                corrupt[entry.name] = entry
                continue
            entries[entry.name] = data
    # A valid duplicate wins in either store order; a corrupt unique entry fails
    # preparation before any output is published, rather than silently losing it.
    for name, entry in corrupt.items():
        if name not in entries:
            raise ValueError(f'corrupt shader artifact: {entry}')
    if not entries:
        raise ValueError('setup produced no portable shader artifacts')
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    files = {}
    for name, data in entries.items():
        (output / name).write_bytes(data)
        files[name] = hashlib.sha256(data).hexdigest()
    (output / 'manifest.json').write_text(json.dumps({
        'version': 1, 'binary_sha256': digest(binary), 'files': files,
    }, indent=2) + '\n')


def package(binary, source, output):
    source = Path(source)
    manifest = json.loads((source / 'manifest.json').read_text())
    if manifest.get('version') != 1 or manifest.get('binary_sha256') != digest(binary):
        raise ValueError('shader bundle must be prepared with the packaged binary')
    files = manifest['files']
    if not files:
        raise ValueError('empty shader bundle')
    for name, expected in files.items():
        if not ENTRY.fullmatch(name) or digest(source / name) != expected:
            raise ValueError(f'invalid shader bundle entry: {name}')
    output = Path(output)
    output.mkdir()
    for name in [*files, 'manifest.json']:
        shutil.copy2(source / name, output / name)
