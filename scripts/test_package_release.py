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
from runtime_platform import check_runtime_platform
import struct

PAYLOAD = b"compiled shader fixture"
def envelope(name):
    return blake3(b'vork-cache-entry-v2\0' + name.encode() + b'\0' + PAYLOAD).digest() + PAYLOAD

ROOT = Path(__file__).resolve().parents[1]


class PackageReleaseTests(unittest.TestCase):
    def test_complete_runtime_must_fit_advertised_floor(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / 'vork'; companion = root / 'compiler'
            engine.write_bytes(b'\x7fELF GLIBC_2.34')
            companion.write_bytes(b'\x7fELF GLIBC_2.35')
            self.assertEqual(check_runtime_platform([engine, companion], 'linux-x64'), (2, 35))
            companion.write_bytes(b'\x7fELF GLIBC_2.39')
            with self.assertRaisesRegex(ValueError, 'compiler requires glibc 2.39'):
                check_runtime_platform([engine, companion], 'linux-x64')
            def macho(major):
                header = bytearray(32)
                struct.pack_into('<I', header, 0, 0xfeedfacf)
                struct.pack_into('<I', header, 16, 1)
                return header + struct.pack('<IIIIII', 0x32, 24, 1, major << 16, 0, 0)
            engine.write_bytes(macho(11)); companion.write_bytes(macho(26))
            self.assertEqual(check_runtime_platform([engine, companion], 'darwin-arm64'), (26, 0))
            companion.write_bytes(macho(27))
            with self.assertRaisesRegex(ValueError, 'compiler requires macOS 27.0'):
                check_runtime_platform([engine, companion], 'darwin-arm64')

    def test_universal_runtime_checks_every_slice(self):
        def macho(major, cpu):
            return struct.pack('<IIIIIIII', 0xfeedfacf, cpu, 0, 6, 1, 24, 0, 0) + struct.pack('<IIIIII', 0x32, 24, 1, major << 16, 0, 0)

        with tempfile.TemporaryDirectory() as tmp:
            companion = Path(tmp) / 'universal.dylib'
            for endian in ('>', '<'):
                for wide in (False, True):
                    fields = 'IIQQII' if wide else 'IIIII'
                    magic = 0xcafebabf if wide else 0xcafebabe
                    for floor in (26, 27):
                        # Put the arm64 slice second, after a valid x86_64 slice.
                        slices = [(0x1000007, macho(11, 0x1000007)), (0x100000c, macho(floor, 0x100000c))]
                        offset = 4096
                        directory = bytearray(struct.pack(endian + 'II', magic, 2))
                        payload = bytearray()
                        for cpu, data in slices:
                            entry = (cpu, 0, offset, len(data), 12) + ((0,) if wide else ())
                            directory.extend(struct.pack(endian + fields, *entry))
                            payload.extend(data.ljust(4096, b'\0'))
                            offset += 4096
                        data = directory.ljust(4096, b'\0') + payload
                        companion.write_bytes(data)
                        with self.subTest(endian=endian, wide=wide, floor=floor):
                            if floor == 26:
                                self.assertEqual(check_runtime_platform([companion], 'darwin-arm64'), (26, 0))
                            else:
                                with self.assertRaisesRegex(ValueError, 'requires macOS 27.0'):
                                    check_runtime_platform([companion], 'darwin-arm64')
                        # Truncating the file inside the second slice must not
                        # let a valid first slice qualify the package.
                        companion.write_bytes(data[:8193])
                        with self.assertRaisesRegex(ValueError, 'invalid universal Mach-O slice'):
                            check_runtime_platform([companion], 'darwin-arm64')
            for data in (b'\xca\xfe\xba\xbe', struct.pack('>II', 0xcafebabe, 0),
                         struct.pack('>II', 0xcafebabe, 0xffffffff), b'\xfe\xed\xfa\xcf'):
                companion.write_bytes(data)
                with self.assertRaisesRegex(ValueError, 'Mach-O'):
                    check_runtime_platform([companion], 'darwin-arm64')

    def test_shader_bundle_excludes_device_artifacts_and_checks_binary(self):
        with tempfile.TemporaryDirectory(prefix='vork-shader-package-') as tmp:
            root = Path(tmp)
            binary = root / 'vork'
            binary.write_bytes(b'binary')
            cache = root / 'cache'
            cache.mkdir()
            shader = 'a' * 64 + '.slang-variant-v1.entry'
            (cache / shader).write_bytes(envelope(shader))
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
            for index,data in enumerate((b'x'*64, envelope(name)[:-1]+b'!', b'x'*32, b'')):
                (cache/name).write_bytes(data)
                output=root/str(index)
                with self.assertRaisesRegex(ValueError,'corrupt shader artifact'):
                    export(binary,[cache],output)
                self.assertFalse(output.exists())

    def test_misplaced_valid_entry_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); binary = root / 'vork'; binary.write_bytes(b'binary')
            cache = root / 'cache'; cache.mkdir()
            name = 'a' * 64 + '.slang-variant-v1.entry'
            other = 'b' * 64 + '.slang-variant-v1.entry'
            (cache / other).write_bytes(envelope(name))
            with self.assertRaisesRegex(ValueError, 'corrupt shader artifact'):
                export(binary, [cache], root / 'output')

    def test_valid_duplicate_wins_in_either_store_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); binary=root/'vork'; binary.write_bytes(b'binary')
            good=root/'good'; good.mkdir(); bad=root/'bad'; bad.mkdir()
            name='a'*64+'.slang-program-v2.entry'
            (good/name).write_bytes(envelope(name)); (bad/name).write_bytes(b'x'*64)
            for index,caches in enumerate(([good,bad],[bad,good])):
                output=root/str(index)
                export(binary,caches,output)
                self.assertEqual((output/name).read_bytes(),envelope(name))
                package(binary,output,root/f'packaged-{index}')

    def test_prepare_shader_command(self):
        with tempfile.TemporaryDirectory(prefix='vork-prepare-test-') as tmp:
            root = Path(tmp)
            binary = root / 'vork'
            binary.write_text(f'#!{sys.executable}\nimport json, sys\nfrom pathlib import Path\n'
                              'Path(__file__).with_suffix(".args").write_text(json.dumps(sys.argv[1:]))\n')
            binary.chmod(0o755)
            cache = root / 'cache'
            cache.mkdir()
            shader = 'a' * 64 + '.slang-variant-v1.entry'
            (cache / shader).write_bytes(envelope(shader))
            subprocess.run([sys.executable, str(ROOT / 'scripts/prepare-shaders'),
                            '--binary', str(binary), '--cache', str(cache),
                            '--output', str(root / 'shaders')], check=True)
            self.assertTrue((root / 'shaders' / shader).is_file())
            command, *scenes = json.loads(binary.with_suffix('.args').read_text())
            self.assertEqual(command, 'prepare-release-shaders')
            self.assertIn(str(ROOT / 'crates/vork-scene/src/starter.json'), scenes)
            if (ROOT / 'scenes').is_dir():
                self.assertTrue({ROOT / 'examples/orbit.json', ROOT / 'examples/jitter-aa.json'} # export-example-private: guarded by private scenes directory
                                .issubset(map(Path, scenes)))
            for scene in scenes:
                self.assertIn('nodes', json.loads(Path(scene).read_text()))

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
            write(root / 'vork-slang-filesystem', '#!/bin/sh\nexit 0\n', True)
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
            (cache / shader).write_bytes(envelope(shader))
            export(root / 'vork', [cache], root / 'shaders')
            subprocess.run([sys.executable, str(ROOT / 'scripts/package-release'), '--binary', str(root / 'vork'),
                            '--platform', 'linux-x64', '--output', str(root / 'out'),
                            '--slang-toolchain', str(slang), '--oidn-toolchain', str(oidn)], check=True)
            archive = next((root / 'out').glob('*.tar.gz'))
            with tarfile.open(archive) as package:
                self.assertIn('vork', package.getnames())
                skill = 'skills/vork/SKILL.md'
                self.assertEqual(package.extractfile(skill).read(), (ROOT / skill).read_bytes())
                self.assertIn('vork-slang-filesystem', package.getnames())
                sdk_metadata = json.loads(package.extractfile('sdk/package.json').read())
                self.assertEqual(sdk_metadata['exports'], {
                    '.': {'types': './index.d.ts', 'import': './index.js'},
                    './scene.schema.json': './scene.schema.json',
                })
                self.assertEqual(sdk_metadata['engines'],
                                 json.loads((ROOT / 'packages/ts/package.json').read_text())['engines'])
                builder = package.extractfile('python/vork/fluid.py').read()
                self.assertEqual(builder, (ROOT / 'packages/python/vork/fluid.py').read_bytes())
                namespace = {}
                exec(compile(builder, 'fluid.py', 'exec'), namespace)
                self.assertTrue(callable(namespace['build']))
                for path in ('sdk/index.js', 'sdk/index.d.ts', 'sdk/scene.schema.json'):
                    self.assertIn(path, package.getnames())
                expected = {line for line in (ROOT / 'dist/example-payload.txt').read_text().splitlines()
                            if line and not line.startswith('#')}
                actual = {entry.name for entry in package.getmembers()
                          if entry.isfile() and entry.name.startswith(('examples/', 'skills/'))}
                self.assertEqual(actual, expected)
                self.assertNotIn('examples/murmuration/scene.json', actual) # export-example-absent: asserts private scene is excluded
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
                self.assertEqual(package.extractfile('shaders/' + shader).read(), envelope(shader))
                path = 'libraries/blender/filmic.exr'
                self.assertEqual(package.extractfile(path).read(), (ROOT / path).read_bytes())
                self.assertNotIn('libraries/std/agx.exr', package.getnames())
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
