#!/usr/bin/env python3
"""CPU-check corpus/example scenes using the permanent std or Blender helper homes.

Usage: python3 scripts/check-library-homes.py /path/to/vork
Follows local JSON/Slang dependencies so external material modules count too.
"""
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
NAMES = re.compile(r'(?:std[/.](?:bloom|procedurals|vector|color|white_balance)|blender[/.](?:procedurals|vector|color|white_balance)|post[/.]white_balance)\b')


def uses_helpers(path, seen):
    path = path.resolve()
    if path in seen or not path.is_file():
        return False
    seen.add(path)
    text = path.read_text()
    if NAMES.search(text):
        return True
    # JSON asset/program references are string values relative to their owner.
    # Only source files can introduce imports; image/buffer assets cannot.
    try:
        value = json.loads(text)
    except ValueError:
        # Authored Slang can delegate to neighbouring modules or textual includes.
        includes = re.findall(r'#include\s+"([^"\n]+)"', text)
        imports = re.findall(r'\bimport\s+([A-Za-z_][\w.]*)\s*;', text)
        dependencies = includes + [name.replace('.', '/') + '.slang' for name in imports]
        return any(uses_helpers(path.parent / name, seen) for name in dependencies)
    def strings(value):
        if isinstance(value, str):
            yield value
        elif isinstance(value, dict):
            for child in value.values():
                yield from strings(child)
        elif isinstance(value, list):
            for child in value:
                yield from strings(child)
    return any(uses_helpers(path.parent / name, seen) for name in strings(value)
               if '\n' not in name and Path(name).suffix in ('.json', '.slang'))


def main():
    binary = str(Path(sys.argv[1]).resolve())
    selected = []
    for folder in ('bench/corpus', 'examples'):
        for path in sorted((ROOT / folder).rglob('*.json')):
            try:
                value = json.loads(path.read_text())
            except ValueError:
                continue
            if isinstance(value, dict) and 'render' in value and uses_helpers(path, set()):
                selected.append(path)
    if not selected:
        raise SystemExit('No helper corpus scenes found')
    failures = []
    for path in selected:
        result = subprocess.run([binary, 'check', str(path)], capture_output=True, text=True)
        print(f'{"PASS" if result.returncode == 0 else "FAIL"} {path.relative_to(ROOT)}', flush=True)
        if result.returncode:
            failures.append(path)
            print(result.stderr, flush=True)
    print(f'{len(selected)} scenes checked; {len(failures)} failures')
    return bool(failures)


if __name__ == '__main__':
    sys.exit(main())
