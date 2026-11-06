# Cozy Kitchen, from Blender

Nicole Morena's Blender **3.5** splash scene demonstrates an imported interior.
[Download the source .blend](https://download.blender.org/demo/splash/blender-3.5-splash.blend)
outside the checkout and read [CREDITS.md](CREDITS.md). Its source designation is
CC BY-SA without a stated version; keep that designation and share alike when
distributing the adaptation. No original asset or movie is stored here.

With Blender 5.2.1 and vork installed, run from the checkout root:

```sh
python3 examples/kitchen/prepare.py /path/to/blender-3.5-splash.blend /tmp/vork-kitchen
vork render /tmp/vork-kitchen/shot.json --profile std/path --at 3 --set resolution=320x180 --out /tmp/kitchen.png
```

The recipe checks the source SHA-256 and material layout, then uses a
shared material program and palette textures in `look/`. This avoids
compiling 177 separate material programs. It approximates the source procedural
materials; noise and bump detail differ. The source export report and
[CREDITS.md](CREDITS.md) record substitutions and changes.

The README image has additional staging and animated lighting that this recipe
does not recreate. Inspect the result and the image before using it.
The Blender importer remains a separate GPL component under
[integrations/blender](../../integrations/blender/). Generated scene files cross
that boundary; the renderer does not import Blender Python.

Use `std/path`: dense transparent interiors can fail in `std/final`, even at low
resolution. This is a demanding scene, not an installation smoke test.
