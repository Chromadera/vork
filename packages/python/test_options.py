import unittest
from vork import std, render, render_arguments
from vork._validate import _check_options


class HelperOptions(unittest.TestCase):
    def test_std_diagnostics(self):
        for generate in (lambda **options: std('sphere', name='ball', **options),):
            with self.assertRaisesRegex(RuntimeError, r'unknown option "raduis"; did you mean "radius"\? Valid names:'):
                generate(raduis=2)
            with self.assertRaises(RuntimeError) as error:
                generate(zzzzzzzz=2)
            self.assertNotIn('did you mean', str(error.exception))
            for name in ('radius', 'rings', 'segments', 'Valid names:'):
                self.assertIn(name, str(error.exception))

    def test_render_keywords_fail_before_invoking_cli(self):
        for function in (render, render_arguments):
            with self.assertRaisesRegex(TypeError, r'unknown option "proflie"; did you mean "profile"\? Nearest valid names'):
                function('missing.json', 'out.png', proflie=[10, 10])
            with self.assertRaises(TypeError) as error:
                function('missing.json', 'out.png', zzzzzzzz=1)
            self.assertNotIn('did you mean', str(error.exception))
            self.assertIn('Nearest valid names', str(error.exception))

    def test_distance_and_display_match_cli(self):
        for key, allowed, expected in (
            ('grb', ['rgb'], 'did you mean "rgb"'),
            ('cot', ['cut', 'cat'], 'did you mean "cat"? Valid names: cat, cut'),
            ('zzzz', ['radius'], 'unknown option "zzzz"; Valid names: radius'),
            ('bad', [], 'unknown option "bad"; no valid names are declared'),
        ):
            with self.assertRaises(TypeError) as error:
                _check_options({key: 1}, allowed)
            self.assertIn(expected, str(error.exception))
