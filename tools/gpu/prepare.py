#!/usr/bin/env python3
"""Create a Docker context from a checksum-verified linux-x64 release."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tarfile


def prepare(bundle, sha256, context):
    with bundle.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    if digest != sha256.lower():
        raise ValueError('release archive SHA256 mismatch')
    context.mkdir(parents=True, exist_ok=False)
    payload = context / 'bundle'
    with tarfile.open(bundle) as archive:
        archive.extractall(payload, filter='data')
    for name in ('vork', 'LICENSE', 'NOTICE', 'licenses', 'shaders', 'slang/bin/slangc',
                 'oidn/lib/libOpenImageDenoise.so.2'):
        if not (payload / name).exists():
            raise ValueError(f'not a complete Linux release: missing {name}')
    if (payload / 'integrations').exists():
        raise ValueError('Blender GPL bridge must not ship in renderer image')
    for name in ('Dockerfile', 'entrypoint.sh', 'runner.py'):
        shutil.copy2(Path(__file__).parent / name, context / name)
    (context / 'entrypoint.sh').chmod(0o755)
    (payload / 'vork').chmod(0o755)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bundle', type=Path, required=True)
    p.add_argument('--sha256', required=True)
    p.add_argument('--context', type=Path, required=True)
    a = p.parse_args()
    prepare(a.bundle, a.sha256, a.context)
