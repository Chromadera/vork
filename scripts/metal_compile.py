"""Compile all MSL witnesses, reusing successful exact-input Apple compiles."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

COMMAND = ['xcrun', '-sdk', 'macosx', 'metal', '-c', '-std=metal3.2']


def compiler_identity():
    commands = [
        ['xcrun', '-sdk', 'macosx', 'metal', '--version'],
        ['xcrun', '-sdk', 'macosx', '--show-sdk-build-version'],
        ['xcodebuild', '-version'],
    ]
    return b'\0'.join([Path(__file__).read_bytes(), platform.machine().encode(),
        repr(COMMAND).encode()] + [subprocess.check_output(c) for c in commands])


def compile_all(shaders):
    cache = os.environ.get('VORK_METAL_WITNESS_CACHE')
    cache = Path(cache) if cache else None
    identity = compiler_identity() if cache else b''
    if cache:
        cache.mkdir(parents=True, exist_ok=True)
    workers = max(1, int(os.environ.get('VORK_METAL_JOBS', '1')))

    def compile_one(shader):
        started = time.monotonic()
        source = shader.read_text().replace('[];', '[1];')
        digest = hashlib.sha256(identity + b'\0' + source.encode()).hexdigest()
        artifact = cache / (digest + '.air') if cache else None
        if artifact and artifact.is_file() and artifact.stat().st_size:
            print(f'Metal cache hit: {shader.name}', flush=True)
            return False
        # Each invocation owns its files, including duplicate source hashes.
        with tempfile.TemporaryDirectory(prefix='vork-metal-') as directory:
            normalized = Path(directory) / shader.name
            normalized.write_text(source)
            output = normalized.with_suffix('.air')
            result = subprocess.run(COMMAND + [str(normalized), '-o', str(output)],
                                    capture_output=True, text=True)
            if result.stdout or result.stderr:
                print(result.stdout + result.stderr, flush=True)
            failed = result.returncode != 0 or not output.is_file() or not output.stat().st_size
            if failed:
                print(f'::error file={shader}::Metal compilation failed: {shader.name}', flush=True)
            elif artifact:
                # Publish only complete successful artifacts, on the same filesystem.
                with tempfile.NamedTemporaryFile(dir=cache, delete=False) as staged:
                    staged.write(output.read_bytes())
                os.replace(staged.name, artifact)
        print(f'Metal compile: {shader.name}: {time.monotonic() - started:.2f}s', flush=True)
        return failed

    with ThreadPoolExecutor(max_workers=workers) as pool:
        return sum(pool.map(compile_one, shaders))
