import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import metal_compile


class WitnessCache(unittest.TestCase):
    def test_exact_inputs_and_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shader = root / 'entry.metal'
            shader.write_text('kernel void entry() {}')
            cache = root / 'cache'
            calls = []
            fail = False

            def compiler(command, **kwargs):
                calls.append(command)
                if not fail:
                    Path(command[-1]).write_bytes(b'compiled AIR')
                return subprocess.CompletedProcess(command, int(fail), '', 'bad source' if fail else '')

            with patch.dict(os.environ, VORK_METAL_WITNESS_CACHE=str(cache), VORK_METAL_JOBS='2'), \
                    patch.object(metal_compile, 'compiler_identity', return_value=b'compiler-sdk-v1') as identity, \
                    patch.object(metal_compile.subprocess, 'run', side_effect=compiler):
                self.assertEqual(metal_compile.compile_all([shader]), 0)
                self.assertEqual(metal_compile.compile_all([shader]), 0)
                self.assertEqual(len(calls), 1)
                shader.write_text('kernel void changed() {}')
                self.assertEqual(metal_compile.compile_all([shader]), 0)
                self.assertEqual(len(calls), 2)
                identity.return_value = b'compiler-sdk-v2'
                self.assertEqual(metal_compile.compile_all([shader]), 0)
                self.assertEqual(len(calls), 3)
                shader.write_text('broken')
                fail = True
                self.assertEqual(metal_compile.compile_all([shader]), 1)
                self.assertEqual(metal_compile.compile_all([shader]), 1)
                self.assertEqual(len(calls), 5)
                self.assertEqual(len(list(cache.glob('*.air'))), 3)

    def test_parallel_batch_preserves_failure_and_normalizes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shaders = [root / f'{i}.metal' for i in range(4)]
            for shader in shaders:
                shader.write_text('float table[];')
            seen = []

            def compiler(command, **kwargs):
                source = Path(command[-3])
                self.assertEqual(source.read_text(), 'float table[1];')
                seen.append(source.name)
                Path(command[-1]).write_bytes(b'AIR')
                return subprocess.CompletedProcess(command, int(source.name == '2.metal'), '', '')

            with patch.dict(os.environ, VORK_METAL_WITNESS_CACHE='', VORK_METAL_JOBS='2'), \
                    patch.object(metal_compile.subprocess, 'run', side_effect=compiler):
                self.assertEqual(metal_compile.compile_all(shaders), 1)
                self.assertCountEqual(seen, [s.name for s in shaders])


if __name__ == '__main__':
    unittest.main()
