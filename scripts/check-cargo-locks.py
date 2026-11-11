#!/usr/bin/env python3
"""Check resolution for every first-party Cargo manifest without rewriting locks."""
from pathlib import Path
import subprocess
import sys


def main():
    root = Path(__file__).resolve().parents[1]
    paths = subprocess.check_output(
        ['git', 'ls-files', '-z', '--', 'Cargo.toml', '**/Cargo.toml'], cwd=root
    ).decode().split('\0')
    failed = False
    for path in sorted(p for p in paths if p and not p.startswith('vendor/')):
        print(f'Checking {path}', flush=True)
        result = subprocess.run(
            ['cargo', 'metadata', '--format-version', '1', '--locked',
             '--manifest-path', path, *sys.argv[1:]],
            cwd=root, stdout=subprocess.DEVNULL,
        )
        failed |= result.returncode != 0
    return int(failed)


if __name__ == '__main__':
    raise SystemExit(main())
