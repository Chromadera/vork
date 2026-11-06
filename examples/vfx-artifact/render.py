#!/usr/bin/env python3
"""Render the opaque graph result to PNGs, then encode outside vork."""
import argparse
from pathlib import Path
import subprocess
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('media', type=Path)
    p.add_argument('--variant', choices=['shot', 'clean'], default='shot')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--vork', default='vork')
    a = p.parse_args()
    media = a.media.resolve()
    checkout = Path(__file__).resolve().parents[2]
    for path in [media, a.out.resolve()]:
        if path == checkout or checkout in path.parents:
            p.error('media and output must be outside the checkout')
    scene = media / ('scene.json' if a.variant == 'shot' else 'clean.json')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    # The transparent CG background is needed for premultiplied over, even
    # though finish.slang returns opaque alpha. The CLI reserves MP4 for
    # scenes with opaque backgrounds, so encode the final graph pixels here.
    with tempfile.TemporaryDirectory(prefix='artifact-frames-', dir=media) as frames:
        subprocess.run([a.vork, 'render', str(scene), '--out', frames + '/'], check=True)
        subprocess.run(['ffmpeg', '-v', 'error', '-y', '-framerate', '25',
                        '-i', str(Path(frames) / '%06d.png'), '-c:v', 'libx264',
                        '-crf', '17', '-preset', 'slow', '-pix_fmt', 'yuv420p',
                        '-movflags', '+faststart', str(a.out)], check=True)


if __name__ == '__main__':
    main()
