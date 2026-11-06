# Python SDK

Ships in every vork release archive as `python/`; install it with
`python3 -m pip install "$PREFIX/lib/vork/python"` (see README.md (Install)), or
`pip install ./packages/python` from the repository. `from vork import Scene, std, write_scene`
exposes the scene type, the shared CLI engines and scene writing:

```python
from vork import Scene, std, write_scene

shape = std('sphere', name='ball', radius=1)
scene: Scene = {'version': 2, **shape,
                'nodes': [{'type': 'mesh', 'geometry': 'ball'},
                          {'type': 'camera', 'translation': [3, 2, 5], 'target': [0, 0, 0]}]}
write_scene(scene, 'ball.json')  # ball.json + its immutable buffer sidecar
```

`write_scene(scene, path, base_dir=None)` mirrors the TypeScript `writeScene`: resource paths
resolve against `base_dir` (default the working directory) and are written relative to the
scene; inline buffers of 4 KiB or more (std shapes, bulky mesh data) go to one
`PATH.buffers.UUID.bin` sidecar. The input is not modified.

`std(function, argument)` or `std(function, **options)` invokes `vork std` using JSON.
Set `VORK_BIN` to choose the executable. Maths, tracks and settings are plain
Python and scene data.

`render('shot.json', 'still.png', at=0, profile='std/previs')` runs `vork render` with render-time
options (`at`, `profile`, `set`, `channels`); the scene file is unchanged.

Regenerate with `scripts/generate-docs`; check with `python3 packages/python/generate.py --check`.

Custom Slang programs declare vertex inputs as `Attribute<float>` through
`Attribute<float4>`. The generated `AttributeBinding` type keeps JSON bindings
such as `{"attribute": "COLOR_0", "channel": "rgb", "factor": [1, 0.5, 1]}`.
`read_attribute(inputs.tint, ctx)` returns value and screen footprints (`dx`, `dy`)
in raster and ray shading. Installation checks names and types on every bound mesh.

Scene writes publish a complete immutable sidecar before atomically replacing the JSON.
Readers therefore see a matching pair, including during concurrent saves. Older sidecars
are retained for readers holding previous JSON snapshots; interrupted saves can also leave
unreferenced sidecars. Fresh files are private; replacement JSON preserves existing permissions.

`render()` prints the CLI exposure lines and returns `{exposure: [{output, exposure}]}`.
Each summary contains `frames`, `coverage` (mean alpha), `clipped`, `near_black`, `mean`, and
`worst_clipped`/`worst_near_black` (`share`, absolute `frame`). EXR and control-only
outputs have no entry. Statistics weight display RGB by alpha; empty coverage gives
null metrics and worst-frame entries. Clip averages exclude empty frames. Values
retain full precision from the output metadata.

Surface programs author emitted light once with OpenPBR `emission_color` and
`emission_luminance`; hits and light samples share that evaluation. Use `lit`
for authored glow that should not become a sampled light.
