#!/usr/bin/env python3
"""Plan a registry publication from tested files: verify, print, never upload.

Shared by `scripts/cut-release publish --dry-run` and the registry-publish
workflow. Every registry file is checked by sha256 against the tested manifest;
a changed, missing or extra file is a refusal (exit 2). The plan lists package,
version, registry, file and sha256 in publication order: native packages before
the wrappers that depend on them.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

NPM_REGISTRY = 'https://registry.npmjs.org'
PYPI_REGISTRY = 'https://upload.pypi.org/legacy/'
NPM_PACKAGES = ('@voyager-so/vork-linux-x64', '@voyager-so/vork-darwin-arm64',
                '@voyager-so/vork-windows-x64', '@voyager-so/vork', 'create-vork')
PYPI_PROJECTS = ('vork-native-linux-x64', 'vork-native-darwin-arm64', 'vork-native-windows-x64', 'vork')
PACKAGES = NPM_PACKAGES + PYPI_PROJECTS


class Mismatch(Exception):
    """The registry files are not the tested files."""


def classify(name, version):
    """(registry, package) for a registry artifact of this version, else None."""
    for package in NPM_PACKAGES:
        if name == package.lstrip('@').replace('/', '-') + f'-{version}.tgz':
            return 'npm', package
    match = re.fullmatch(r'([A-Za-z0-9_]+)-' + re.escape(version) + r'-py3-none-[A-Za-z0-9_.]+\.whl', name)
    if match and match[1].replace('_', '-') in PYPI_PROJECTS:
        return 'pypi', match[1].replace('_', '-')
    return None


def plan(directory, manifest, version):
    """Verified rows for every tested registry file in directory, in publication order."""
    directory = Path(directory)
    if manifest.get('version') != version:
        raise Mismatch(f'tested manifest is for version {manifest.get("version")}, not {version}')
    tested = {n: h for n, h in manifest.get('files', {}).items() if classify(n, version)}
    present = {p.name for p in directory.iterdir() if classify(p.name, version)}
    problems = [f'{n}: present but not in the tested manifest' for n in sorted(present - set(tested))]
    problems += [f'{n}: tested but missing' for n in sorted(set(tested) - present)]
    named = {classify(n, version)[1] for n in tested}
    problems += [f'{p}: the tested manifest has no file for this package' for p in PACKAGES if p not in named]
    rows = []
    for name, expected in sorted(tested.items()):
        if name not in present:
            continue
        actual = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        if actual != expected:
            problems.append(f'{name}: sha256 {actual} is not the tested {expected}')
            continue
        registry, package = classify(name, version)
        rows.append(dict(package=package, version=version, file=name, sha256=actual,
                         registry=NPM_REGISTRY if registry == 'npm' else PYPI_REGISTRY))
    if problems:
        raise Mismatch('Registry files are not the tested files:\n  ' + '\n  '.join(problems))
    rows.sort(key=lambda row: PACKAGES.index(row['package']))
    return rows


def render(rows):
    return '\n'.join(f'{row["package"]} {row["version"]} -> {row["registry"]}\n'
                     f'    {row["file"]}  sha256 {row["sha256"]}' for row in rows)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--dist', type=Path, required=True, help='directory holding the registry files')
    p.add_argument('--manifest', type=Path, required=True, help='tested manifest with sha256 per file')
    p.add_argument('--version', required=True)
    p.add_argument('--format', choices=('text', 'json'), default='text')
    a = p.parse_args()
    try:
        rows = plan(a.dist, json.loads(a.manifest.read_text()), a.version)
    except Mismatch as error:
        print(f'REFUSED: {error}', file=sys.stderr)
        sys.exit(2)
    if a.format == 'json':
        print(json.dumps(rows, indent=2))
    else:
        print(render(rows))
        print(f'{len(rows)} registry files are the tested files. Nothing uploaded.')


if __name__ == '__main__':
    main()
