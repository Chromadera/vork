#!/usr/bin/env python3
"""Plan a registry publication from tested files: verify and print; upload only with --publish-npm.

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
import subprocess
import time

NPM_REGISTRY = 'https://registry.npmjs.org'
PYPI_REGISTRY = 'https://upload.pypi.org/legacy/'
NPM_PACKAGES = ('@voyager-so/vork-linux-x64', '@voyager-so/vork-darwin-arm64',
                '@voyager-so/vork-windows-x64', '@voyager-so/vork', 'create-vork')
PYPI_PROJECTS = ('vork',)
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


def wait_npm_visible(row, timeout=1200, interval=15):
    """Require exact-version metadata and a reachable tarball within 20 minutes."""
    spec = f'{row["package"]}@{row["version"]}'
    deadline = time.monotonic() + timeout
    last_error = 'no successful registry response'

    def run(command):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('visibility deadline reached')
        return subprocess.run(command, check=True, capture_output=True, text=True,
                              timeout=min(60, remaining)).stdout.strip()

    while time.monotonic() < deadline:
        try:
            command = ['npm', 'view', spec, '--registry', NPM_REGISTRY, '--prefer-online']
            version = json.loads(run(command + ['version', '--json']))
            if version != row['version']:
                raise ValueError(f'expected {row["version"]}, got {version!r}')
            tarball = json.loads(run(command + ['dist.tarball', '--json']))
            if not isinstance(tarball, str) or not tarball.startswith('https://'):
                raise ValueError(f'invalid tarball URL: {tarball!r}')
            run(['curl', '--fail', '--silent', '--show-error', '--location', '--head',
                 '--proto', '=https', '--proto-redir', '=https', tarball])
            print(f'VISIBLE: {spec} (metadata and tarball)', flush=True)
            return
        except (subprocess.SubprocessError, OSError, ValueError, TimeoutError) as error:
            last_error = str(error)
        remaining = deadline - time.monotonic()
        if remaining > 0:
            print(f'Waiting for {spec}: {last_error}', flush=True)
            time.sleep(min(interval, remaining))
    raise Mismatch(f'npm visibility timed out after {timeout}s for {spec}: {last_error}')


def publish_npm(rows, directory):
    """Publish in dependency order, waiting before any dependant can upload."""
    npm_rows = sorted((row for row in rows if row['registry'] == NPM_REGISTRY),
                      key=lambda row: NPM_PACKAGES.index(row['package']))
    for row in npm_rows:
        subprocess.run(['npm', 'publish', str(Path(directory).resolve() / row['file']),
                        '--ignore-scripts', '--access', 'public', '--tag', 'latest',
                        '--provenance', '--registry', NPM_REGISTRY], check=True)
        # Also wait for the SDK before create-vork, which pins it exactly.
        wait_npm_visible(row)
    missing = []
    for row in npm_rows:
        try:
            wait_npm_visible(row)
        except Mismatch as error:
            missing.append(str(error))
    if missing:
        raise Mismatch('Final npm visibility check failed:\n  ' + '\n  '.join(missing))
    print(f'Final npm visibility check passed: {len(npm_rows)} packages.', flush=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--dist', type=Path, required=True, help='directory holding the registry files')
    p.add_argument('--manifest', type=Path, required=True, help='tested manifest with sha256 per file')
    p.add_argument('--version', required=True)
    p.add_argument('--format', choices=('text', 'json'), default='text')
    p.add_argument('--publish-npm', action='store_true',
                   help='publish verified npm files and wait for registry visibility')
    a = p.parse_args()
    try:
        rows = plan(a.dist, json.loads(a.manifest.read_text()), a.version)
        if a.publish_npm:
            publish_npm(rows, a.dist)
            return
    except (Mismatch, subprocess.CalledProcessError) as error:
        print(f'REFUSED: {error}', file=sys.stderr)
        sys.exit(2)
    if a.format == 'json':
        print(json.dumps(rows, indent=2))
    else:
        print(render(rows))
        print(f'{len(rows)} registry files are the tested files. Nothing uploaded.')


if __name__ == '__main__':
    main()
