# OCIO display transforms

The sole AgX sample asset is `libraries/blender/agx.exr`. `std/agx` reads its
neutral band and nine look bands.
`agx-manifest.json` retains the provenance of the original neutral samples.
Only mathematical transform samples are imported; rendering and GPU integration
remain native vork/wgpu. See `libraries/blender/agx-manifest.json` for the tiled
65³ half-float layout and `scripts/color/bake-agx-looks.py` for regeneration.
