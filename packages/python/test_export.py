"""Export SDK process-boundary contract; no GPU required."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import vork


class ExportTests(unittest.TestCase):
    def test_selection_time_losses_and_report(self):
        with tempfile.TemporaryDirectory() as folder:
            asset = str(Path(folder) / 'hero.glb')
            report = dict(version=1, status='complete', issues=[])
            Path(folder, 'hero.export.json').write_text(json.dumps(report))
            with patch.object(vork, '_run_engine') as run:
                self.assertEqual(vork.export_asset('scene.json', asset, root='hero', at=1.25,
                                                  allow_losses=['camera_dof']), report)
                run.assert_called_once_with(['export', 'scene.json', '--out', asset,
                                             '--root', 'hero', '--at', '1.25',
                                             '--allow-loss', 'camera_dof'], check=True)

    def test_animation_range(self):
        with tempfile.TemporaryDirectory() as folder:
            asset = str(Path(folder) / 'clip.glb')
            Path(folder, 'clip.export.json').write_text('{"status":"complete"}')
            with patch.object(vork, '_run_engine') as run:
                vork.export_asset('scene.json', asset, frame_start=12, frame_end=36)
                run.assert_called_once_with(['export', 'scene.json', '--out', asset,
                                             '--frame-start', '12', '--frame-end', '36'], check=True)


if __name__ == '__main__':
    unittest.main()
