# SPDX-License-Identifier: GPL-3.0-or-later
"""Write scene-linear World pixels without Blender's image-save color conversion."""
from pathlib import Path


def write_hdr(path, rgb):
    import numpy as np

    rgb = np.asarray(rgb, dtype=np.float32)
    if rgb.ndim != 3 or rgb.shape[2] != 3:
        raise ValueError('World radiance must be H×W×RGB')
    if not np.isfinite(rgb).all() or (rgb < 0).any():
        raise ValueError('World radiance must be finite and nonnegative')
    maximum = rgb.max(axis=2)
    mantissa, exponent = np.frexp(maximum)
    scale = np.divide(mantissa * 256, maximum,
                      out=np.zeros_like(maximum), where=maximum > 1e-32)
    rgbe = np.zeros((*rgb.shape[:2], 4), dtype=np.uint8)
    rgbe[:, :, :3] = np.minimum(rgb * scale[:, :, None], 255).astype(np.uint8)
    rgbe[:, :, 3] = np.where(maximum > 1e-32, exponent + 128, 0).astype(np.uint8)
    height, width = rgb.shape[:2]
    # Radiance permits flat RGBE pixels as well as scanline RLE. Blender's pixel
    # buffer starts at the bottom; -Y files start at the top.
    with Path(path).open('wb') as output:
        output.write(f'#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n'.encode())
        output.write(rgbe[::-1].tobytes())


def write_image(path, image):
    import numpy as np

    width, height = image.size
    pixels = np.empty(width * height * image.channels, dtype=np.float32)
    image.pixels.foreach_get(pixels)
    write_hdr(path, pixels.reshape(height, width, image.channels)[:, :, :3])
