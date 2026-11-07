"""Resolve the project-local native package; archive SDKs retain PATH support."""
import os
import platform
from importlib.util import find_spec
from pathlib import Path
import subprocess
import sys
import threading


def engine(require_native=False):
    if os.environ.get('VORK_BIN'):
        return os.environ['VORK_BIN']
    target = {('Linux', 'x86_64'): 'linux_x64', ('Darwin', 'arm64'): 'darwin_arm64',
              ('Windows', 'AMD64'): 'windows_x64'}.get((platform.system(), platform.machine()))
    if target is None:
        raise RuntimeError(f'Unsupported vork platform: {platform.system()} {platform.machine()}')
    spec = find_spec('vork_native_' + target)
    if spec and spec.origin:
        return str(Path(spec.origin).parent / 'runtime' / ('vork.exe' if os.name == 'nt' else 'vork'))
    if require_native:
        raise RuntimeError('native package missing; reinstall vork with dependencies from your package index')
    return 'vork'


def main():
    args = sys.argv[1:]
    if args and args[0] in ('setup', 'render'):
        print('vork: GPU shader compilation can take several minutes on first use. '
              'Run `vork setup` after upgrading or changing drivers.', file=sys.stderr, flush=True)
    stop = threading.Event()
    def progress():
        while not stop.wait(15):
            print('vork: setup is still compiling GPU shaders…', file=sys.stderr, flush=True)
    if args and args[0] == 'setup':
        threading.Thread(target=progress, daemon=True).start()
    try:
        result = subprocess.run([engine(require_native=True), *args])
        return result.returncode if result.returncode >= 0 else 128 - result.returncode
    except (FileNotFoundError, RuntimeError) as error:
        print(f'vork: {error}', file=sys.stderr)
        return 1

    finally:
        stop.set()
