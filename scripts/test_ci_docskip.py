import importlib.util
import unittest
import os
import subprocess
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('docskip', 'scripts/ci-docskip.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DocumentationTests(unittest.TestCase):
    def test_documentation(self):
        for path in ['README.md', 'out/report.md', 'docs/help/topic.txt', 'notes/a.txt', 'images/diagram.png']:
            with self.subTest(path=path):
                self.assertTrue(module.documentation(path))

    def test_code_wins(self):
        for path in ['crates/README.md', 'libraries/image.png', 'bench/notes.md', 'scripts/docs.md', '.github/README.md', 'Cargo.lock', 'scene.json', 'out/report.txt', 'examples/assets/brick.png', 'diagram.svg']:
            with self.subTest(path=path):
                self.assertFalse(module.documentation(path))


class RangeTests(unittest.TestCase):
    def classify(self, paths, pr='false'):
        with patch.dict(os.environ, BEFORE='abc', HEAD='def', IS_PR=pr):
            with patch.object(subprocess, 'check_output', side_effect=paths) as run:
                result = module.code_changed()
                return result, run.call_args_list

    def test_push_and_pr_ranges(self):
        self.assertFalse(self.classify([b'README.md\0docs/help.txt\0'])[0])
        result, calls = self.classify(['base\n', b'README.md\0scripts/code.py\0'], 'true')
        self.assertTrue(result)
        self.assertEqual(calls[0].args[0], ['git', 'merge-base', 'abc', 'def'])
        self.assertIn('--no-renames', calls[1].args[0])
        self.assertIn('base', calls[1].args[0])

    def test_empty_or_unavailable_range_runs_code(self):
        self.assertTrue(self.classify([b''])[0])
        self.assertTrue(self.classify([subprocess.CalledProcessError(1, 'git')])[0])
        with patch.dict(os.environ, BEFORE='0' * 40, HEAD='def'):
            self.assertTrue(module.code_changed())

if __name__ == '__main__':
    unittest.main()
