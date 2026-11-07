#!/usr/bin/env python3
"""Batch transport shared by user-account GPU templates. No listening service."""
import argparse
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time

MAX_BYTES = 512 * 1024 * 1024


def pack_job(directory):
    directory = directory.resolve()
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode='w:gz') as archive:
        for path in sorted(directory.rglob('*')):
            if path.is_symlink():
                raise ValueError(f'job symlinks are not supported: {path}')
            if path.is_file():
                archive.add(path, arcname=path.relative_to(directory), recursive=False)
    if buf.tell() > MAX_BYTES:
        raise ValueError('job archive exceeds 512 MiB; use a smaller self-contained job')
    return buf.getvalue()


def unpack(data, destination):
    if len(data) > MAX_BYTES:
        raise ValueError('artifact archive exceeds 512 MiB')
    destination.mkdir(parents=True, exist_ok=False)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        archive.extractall(destination, filter='data')


def execute(payload, scene, options, runs, timeout):
    if Path(scene).is_absolute() or '..' in Path(scene).parts:
        raise ValueError('--scene must be relative to --job')
    if any(x == '--out' or x.startswith('--out=') for x in options):
        raise ValueError('output is managed by the template; do not pass --out')
    with tempfile.TemporaryDirectory(prefix='vork-job-') as work:
        root = Path(work)
        unpack(payload, root / 'job')
        output = root / 'results'
        output.mkdir()
        cache = root / 'cache'
        cache.mkdir()
        import os
        env = dict(os.environ, VORK_CACHE_DIR=str(cache))
        gpu = subprocess.run(['nvidia-smi'], capture_output=True, text=True)
        (output / 'gpu.txt').write_text(gpu.stdout + gpu.stderr)
        records = []
        for index in range(runs):
            target = output / f'run-{index+1}'
            target.mkdir()
            command = ['vork', 'render', scene, '--out', str(target / 'video.mp4'),
                       '--diagnostics', *options]
            print(f'Run {index+1}/{runs}: rendering {scene}; log returns with artifacts', flush=True)
            started = time.monotonic()
            with (target / 'render.log').open('w') as log:
                try:
                    result = subprocess.run(command, cwd=root / 'job', env=env,
                                            stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
                    code = result.returncode
                except subprocess.TimeoutExpired:
                    code = 124
            elapsed = time.monotonic() - started
            probe = subprocess.run(['ffprobe', '-v', 'error', '-show_streams', '-show_format',
                                    '-of', 'json', str(target / 'video.mp4')], capture_output=True, text=True)
            (target / 'ffprobe.json').write_text(probe.stdout or '{}')
            if code == 0 and probe.returncode:
                code = 4
            records.append(dict(run=index+1, seconds=elapsed, exit_code=code,
                                command=command, probe_error=probe.stderr))
            print(json.dumps(records[-1]), flush=True)
            if code:
                break
        (output / 'job.json').write_text(json.dumps(records, indent=2) + '\n')
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode='w:gz') as archive:
            for path in sorted(output.rglob('*')):
                if path.is_file():
                    archive.add(path, arcname=path.relative_to(output), recursive=False)
        if buf.tell() > MAX_BYTES:
            raise ValueError('results exceed 512 MiB; render a shorter clip')
        return records[-1]['exit_code'], buf.getvalue()


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('input', type=Path)
    p.add_argument('output', type=Path)
    p.add_argument('--scene', required=True)
    p.add_argument('--runs', type=int, default=1)
    p.add_argument('--timeout', type=int, default=1800)
    # Options after -- belong to vork, rather than to this transport.
    import sys
    argv = sys.argv[1:]
    split = argv.index('--') if '--' in argv else len(argv)
    a = p.parse_args(argv[:split])
    if not 1 <= a.runs <= 10 or a.timeout < 1:
        p.error('runs must be 1..10 and timeout positive')
    code, data = execute(a.input.read_bytes(), a.scene, argv[split+1:], a.runs, a.timeout)
    a.output.write_bytes(data)
    raise SystemExit(code)
