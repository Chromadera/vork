"""Offline contract for the release advice copied into npm package metadata."""
import importlib.machinery
import importlib.util
from pathlib import Path
import unittest

loader = importlib.machinery.SourceFileLoader('package_registries', str(Path(__file__).with_name('package-registries')))
spec = importlib.util.spec_from_loader(loader.name, loader)
package = importlib.util.module_from_spec(spec)
loader.exec_module(package)


class UpdateAdviceTests(unittest.TestCase):
    def test_optional_release_advice(self):
        self.assertEqual(package.update_advice({}), {})
        advice = {'reason': 'fixes black frames in Windows fog renders', 'regression': True}
        self.assertEqual(package.update_advice({'vorkUpdate': advice}), advice)

    def test_reject_invalid_or_multiline_release_advice(self):
        for advice in [None, [], {'reason': 4}, {'reason': 'x' * 241},
                       {'reason': 'two\nlines'}, {'reason': '\x1b[31m'},
                       {'regression': 'yes'}, {'url': 'https://elsewhere'}]:
            with self.subTest(advice=advice), self.assertRaises(ValueError):
                package.update_advice({'vorkUpdate': advice})


if __name__ == '__main__':
    unittest.main()
