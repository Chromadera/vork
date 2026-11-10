import runpy
from pathlib import Path
import unittest
import subprocess
import sys
import tempfile

scan = runpy.run_path(str(Path(__file__).with_name('check-metal-generated')))['file_scope_acceleration_structures']
AS = 'metal::raytracing::acceleration_structure<metal::raytracing::instancing>'


class ScopeScan(unittest.TestCase):
    def test_illegal_global(self):
        self.assertEqual(len(scan(f'{AS} ray_scene_0;')), 1)
        self.assertEqual(len(scan(f'struct Context {{ {AS} ray_scene_1; }};\n{AS}\nray_scene_0;')), 1)

    def test_initialized_and_qualified_globals(self):
        self.assertEqual(scan(f'{AS} const global = {{}}; {AS} *pointer;'), ['global', 'pointer'])

    def test_legal_context_and_parameter(self):
        self.assertEqual(scan(f'struct Context {{ {AS} ray_scene_0; }};\nvoid entry({AS} ray_scene_0) {{}}'), [])

    def test_comments_and_strings(self):
        self.assertEqual(scan(f'// {AS} bad;\n/* {{ */\nconst char* s="{{}}";\n{AS} bad;'), ['bad'])

    def test_preprocessor_does_not_hide_global(self):
        self.assertEqual(len(scan(f'#include <metal_stdlib>\n{AS} bad;')), 1)

    def test_function_prototype_is_not_variable(self):
        self.assertEqual(scan(f'void helper({AS} ray_scene_0);'), [])


class ScanCommand(unittest.TestCase):
    def test_empty_module_is_not_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, 'placeholder.metal').touch()
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('check-metal-generated')),
                '--scan', directory], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('empty MSL module', result.stderr)

    def test_stale_output_is_refused_before_building(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, 'old.metal').write_text('kernel void old() {}')
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('check-metal-generated')),
                '--output', directory], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('output already contains MSL', result.stderr)


if __name__ == '__main__':
    unittest.main()
