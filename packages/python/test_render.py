import json
import os
import tempfile
import unittest
from unittest.mock import patch
from vork import render


class RenderExposure(unittest.TestCase):
    def test_returns_full_precision_per_output_and_keeps_cli_stdout_visible(self):
        summary = {'frames': 2, 'coverage': 0.5, 'clipped': 0.123456789, 'near_black': 0.2, 'mean': 0.41,
                   'worst_clipped': {'share': 0.25, 'frame': 7},
                   'worst_near_black': {'share': 0.3, 'frame': 8}}
        empty = dict(frames=1, coverage=0, clipped=None, near_black=None, mean=None,
                     worst_clipped=None, worst_near_black=None)
        summaries = [summary, empty, summary]
        warning = dict(code="additional_environment", requested={"node": {"name": "second", "energy": 2}}, dropped="second environment; retaining first")
        with tempfile.TemporaryDirectory() as directory:
            outputs = [os.path.join(directory, p) for p in ['still.png', 'clip.MOV', 'frames.png/', 'controls', 'exr/%06d.exr']]
            paths = ['still.png.meta.json', 'clip.MOV.meta.json', 'frames.png/meta.json', 'controls/meta.json', 'exr/meta.json']
            for i, path in enumerate(paths):
                path = os.path.join(directory, path)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, 'w') as f:
                    json.dump({'exposure': summaries[i], 'warnings': [warning]} if i < 3 else {'warnings': [warning]}, f)
            with patch('subprocess.run') as run:
                result = render('shot.json', outputs)
                self.assertEqual(run.call_args.kwargs, {'check': True})
            self.assertEqual(result, {'exposure': [{'output': out, 'exposure': summaries[i]} for i, out in enumerate(outputs[:3])], 'warnings': [warning]})

    def test_01_metadata_without_warnings_preserves_exposure(self):
        summary = {'frames': 1, 'coverage': 1, 'mean': 0.4}
        with tempfile.TemporaryDirectory() as directory:
            output = os.path.join(directory, 'still.png')
            with open(output + '.meta.json', 'w') as f:
                json.dump({'exposure': summary}, f)
            with patch('subprocess.run'):
                self.assertEqual(render('shot.json', output), {
                    'exposure': [{'output': output, 'exposure': summary}], 'warnings': []})


if __name__ == '__main__':
    unittest.main()
