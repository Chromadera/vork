#!/usr/bin/env python3
"""Exercise release archive contents with local fixture toolchains; no downloads."""
import hashlib
import json
from pathlib import Path
import subprocess
import shutil
import tarfile
import tempfile
import tomllib
import unittest
import sys
from blake3 import blake3
from shader_bundle import export, package

PAYLOAD = b"compiled shader fixture"
ENVELOPE = blake3(PAYLOAD).digest() + PAYLOAD

ROOT = Path(__file__).resolve().parents[1]


class PackageReleaseTests(unittest.TestCase):
    def test_shader_bundle_excludes_device_artifacts_and_checks_binary(self):
        with tempfile.TemporaryDirectory(prefix='vork-shader-package-') as tmp:
            root = Path(tmp)
            binary = root / 'vork'
            binary.write_bytes(b'binary')
            cache = root / 'cache'
            cache.mkdir()
            shader = 'a' * 64 + '.slang-variant-v1.entry'
            (cache / shader).write_bytes(ENVELOPE)
            (cache / ('b' * 64 + '.pipeline-v1.entry')).write_bytes(b'driver')
            export(binary, [cache], root / 'bundle')
            package(binary, root / 'bundle', root / 'shaders')
            self.assertEqual(sorted(p.name for p in (root / 'shaders').iterdir()), [shader, 'manifest.json'])
            binary.write_bytes(b'other binary')
            with self.assertRaisesRegex(ValueError, 'packaged binary'):
                package(binary, root / 'bundle', root / 'wrong')
            binary.write_bytes(b'binary')
            (root / 'bundle' / shader).write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError, 'invalid shader bundle entry'):
                package(binary, root / 'bundle', root / 'corrupt')

    def test_corrupt_cache_envelopes_fail_before_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); binary=root/'vork'; binary.write_bytes(b'binary')
            cache=root/'cache'; cache.mkdir()
            name='a'*64+'.slang-variant-v1.entry'
            for index,data in enumerate((b'x'*64, ENVELOPE[:-1]+b'!', b'x'*32, b'')):
                (cache/name).write_bytes(data)
                output=root/str(index)
                with self.assertRaisesRegex(ValueError,'corrupt shader artifact'):
                    export(binary,[cache],output)
                self.assertFalse(output.exists())

    def test_valid_duplicate_wins_in_either_store_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); binary=root/'vork'; binary.write_bytes(b'binary')
            good=root/'good'; good.mkdir(); bad=root/'bad'; bad.mkdir()
            name='a'*64+'.slang-program-v2.entry'
            (good/name).write_bytes(ENVELOPE); (bad/name).write_bytes(b'x'*64)
            for index,caches in enumerate(([good,bad],[bad,good])):
                output=root/str(index)
                export(binary,caches,output)
                self.assertEqual((output/name).read_bytes(),ENVELOPE)
                package(binary,output,root/f'packaged-{index}')

    def test_prepare_shader_command(self):
        with tempfile.TemporaryDirectory(prefix='vork-prepare-test-') as tmp:
            root = Path(tmp)
            binary = root / 'vork'
            binary.write_text('#!/bin/sh\ntest \"$1\" = prepare-release-shaders\n')
            binary.chmod(0o755)
            cache = root / 'cache'
            cache.mkdir()
            shader = 'a' * 64 + '.slang-variant-v1.entry'
            (cache / shader).write_bytes(ENVELOPE)
            subprocess.run([sys.executable, str(ROOT / 'scripts/prepare-shaders'),
                            '--binary', str(binary), '--cache', str(cache),
                            '--output', str(root / 'shaders')], check=True)
            self.assertTrue((root / 'shaders' / shader).is_file())

    def test_filmic_ships_beside_executable(self):
        with tempfile.TemporaryDirectory(prefix='vork-package-test-') as tmp:
            root = Path(tmp)

            def write(path, text='fixture', executable=False):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
                if executable:
                    path.chmod(0o755)

            version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
            write(root / 'vork', f'#!/bin/sh\necho vork {version}\n', True)
            slang = root / 'slang'
            pin = json.loads((ROOT / 'dist/slang.json').read_text())['version']
            write(slang / 'bin/slangc', f'#!/bin/sh\necho {pin} >&2\n', True)
            for name in ['LICENSE', 'LICENSES/fixture', 'third-party-notices/fixture',
                         'lib/libslang-compiler.so.fixture', 'lib/libslang-glsl-module-fixture.so',
                         'lib/libslang-glslang-fixture.so', 'lib/slang-standard-module-fixture']:
                write(slang / name)
            oidn = root / 'oidn'
            pin = json.loads((ROOT / 'dist/oidn.json').read_text())
            write(oidn / 'vork-oidn.json', json.dumps({'version': pin['version'], 'platform': 'linux-x64',
                  'sha256': pin['archives']['linux-x64']['sha256']}))
            for name in ['doc/LICENSE.txt', 'doc/third-party-programs.txt', 'lib/libOpenImageDenoise.so.2', 'bin/fixture']:
                write(oidn / name)
            cache = root / 'cache'
            cache.mkdir()
            shader = 'a' * 64 + '.slang-variant-v1.entry'
            (cache / shader).write_bytes(ENVELOPE)
            export(root / 'vork', [cache], root / 'shaders')
            subprocess.run([sys.executable, str(ROOT / 'scripts/package-release'), '--binary', str(root / 'vork'),
                            '--platform', 'linux-x64', '--output', str(root / 'out'),
                            '--slang-toolchain', str(slang), '--oidn-toolchain', str(oidn)], check=True)
            archive = next((root / 'out').glob('*.tar.gz'))
            with tarfile.open(archive) as package:
                self.assertIn('vork', package.getnames())
                sdk_metadata = json.loads(package.extractfile('sdk/package.json').read())
                self.assertEqual(sdk_metadata['exports'], {
                    '.': {'types': './index.d.ts', 'import': './index.js'},
                    './scene.schema.json': './scene.schema.json',
                })
                self.assertEqual(sdk_metadata['engines'],
                                 json.loads((ROOT / 'packages/ts/package.json').read_text())['engines'])
                for path in ('sdk/index.js', 'sdk/index.d.ts', 'sdk/scene.schema.json'):
                    self.assertIn(path, package.getnames())
                expected = {line for line in (ROOT / 'dist/example-payload.txt').read_text().splitlines()
                            if line and not line.startswith('#')}
                actual = {entry.name for entry in package.getmembers()
                          if entry.isfile() and entry.name.startswith('examples/')}
                self.assertEqual(actual, expected)
                self.assertNotIn('examples/murmuration/scene.json', actual)
                self.assertIn('examples/murmuration/launch/skim.json.in', actual)
                self.assertIn('examples/cake/scene.json.in', actual)
                for name in ('vignette.slang', 'chromatic-dispersion.slang', 'glare.slang',
                             'highlight.slang', 'glare-streaks.json'):
                    self.assertIn('examples/compositor-effects/' + name, actual)
                for path in ('LICENSE', 'sdk/LICENSE', 'python/LICENSE'):
                    self.assertEqual(package.extractfile(path).read(), (ROOT / 'LICENSE').read_bytes())
                self.assertNotIn('LICENSE-MIT', package.getnames())
                self.assertNotIn('LICENSE-APACHE', package.getnames())
                self.assertFalse(any(p.startswith('integrations/') for p in package.getnames()))
                self.assertEqual(package.extractfile('shaders/' + shader).read(), ENVELOPE)
                path = 'libraries/blender/filmic.exr'
                self.assertEqual(package.extractfile(path).read(), (ROOT / path).read_bytes())
                self.assertNotIn('libraries/blender/agx.exr', package.getnames())
            # A hosted job must never silently produce a cold archive.
            shutil.rmtree(root / 'shaders')
            missing = subprocess.run([sys.executable, str(ROOT / 'scripts/package-release'),
                '--binary', str(root / 'vork'), '--platform', 'linux-x64', '--output', str(root / 'missing'),
                '--slang-toolchain', str(slang), '--oidn-toolchain', str(oidn)], capture_output=True, text=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn('portable shaders are required', missing.stderr)
            self.assertIn(hashlib.sha256(archive.read_bytes()).hexdigest(),
                          (root / 'out/SHA256SUMS').read_text())


if __name__ == '__main__':
    unittest.main()
