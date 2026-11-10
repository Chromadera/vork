#!/usr/bin/env python3
"""Conservatively classify the full PR diff (or push range) for CI workers."""
import os
import subprocess


def documentation(path):
    if path.startswith(('crates/', 'libraries/', 'bench/', 'scripts/', '.github/')):
        return False
    return path.endswith('.md') or path.startswith(('docs/', 'notes/', 'images/'))


def code_changed():
    before, head = os.environ['BEFORE'], os.environ['HEAD']
    if not before or set(before) == {'0'}:
        return True
    try:
        if os.environ.get('IS_PR') == 'true':
            before = subprocess.check_output(['git', 'merge-base', before, head], text=True).strip()
        paths = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', '-z', before, head]).decode().split('\0')
    except (subprocess.CalledProcessError, UnicodeError):
        return True
    paths = [path for path in paths if path]
    return not paths or any(not documentation(path) for path in paths)


if __name__ == '__main__':
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'code={str(code_changed()).lower()}\n')
