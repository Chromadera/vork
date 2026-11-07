# Rendering benchmarks

**Draft and final measurements are recorded in [RESULTS.md](RESULTS.md).** Preview and path timings there are explicitly carried forward.

The ten small scenes cover blocking, a moving car proxy, crowd, interior,
night lights, camera push-in, rack focus, street, turntable and walking animation.
Procedural GLBs in
`assets/` are CC0; the animated Fox retains its attribution in
[assets/README.md](assets/README.md). The turntable downloads hash-pinned ToyCar
assets with `python3 benchmarks/fetch.py`. All other inputs are already included.
The car is a proxy, not a photographed car or a production vehicle asset.

## Rerun

Use an installed build and a real supported GPU, then run from the checkout root:

```sh
python3 benchmarks/fetch.py
python3 benchmarks/run.py --vork vork --out /tmp/vork-benchmark
```

The runner resolves named profiles but forces `denoise=false`. By default it
selects only B scenes and previs/final/path, omits draft, and does not set seed 11.
It does not reproduce the historical denoised final8 tables or their C/D scenes.
Those measurements use build `81ae2b075` and the settings listed with each table.

It records setup, a separate per-scene warmup, five warm process
times, commands, exit codes, diagnostics and PNGs. Setup uses the existing cache;
this is not a cold-cache benchmark. Use a new `VORK_CACHE_DIR` to measure first use
and record that choice. `--resolution 320x180 --repetitions 1` is a smoke run only.
`std/previs` runs are previews, with different lighting/shadows; report them
separately from final/path. No timings are projections. Inspect every image and
report failures, hardware, driver, build, cache state and quality differences.

**Known failure:** the helper fails during camera preparation on Blender 5.2.1:
its Track To axis enum is invalid, and Python exceptions do not stop preparation
reliably. The historical comparisons cannot currently be reproduced with it.
Repair and validation across scenes are pending. The commands below describe
the intended workflow.

To create and time Blender inputs, use the separately licensed
[Blender benchmark helper](../integrations/blender/benchmark.py). It imports the
same scene assets and copies the camera and lights from these scene files. Its
import/save preparation is outside timing; each render launches a fresh process.
Run it with system Python (the helper launches Blender):

```sh
python3 integrations/blender/benchmark.py --blender blender --out /tmp/blender-benchmark
```

Use `--cycles-device METAL` on Apple Silicon or `CUDA` if OptiX is unavailable.
The helper renders Blender frame 0 to match vork time 0 by default. It also
translates the supplied camera/material tracks and clip selection for full-video
runs. glTF material translation, shutter integration and renderer sampling still
differ. Re-measure both tools together; do not mix results from different builds or settings.


## Full preview clips

Both runners accept `--scenes B-blocking,B-push-in` to select inputs and `--video`
to render all 240 frames. The default is every `B-*.json`. Run previews separately:

```sh
python3 benchmarks/run.py --profiles previs --video --out /tmp/vork-preview
python3 integrations/blender/benchmark.py --engines BLENDER_EEVEE,BLENDER_WORKBENCH --video --out /tmp/blender-preview
```

EEVEE uses 64 samples and ray tracing; Workbench is a studio preview without the
same lighting or shadows. Encoding paths differ. Keep these qualifications next
to any ratio; numerical settings do not establish equal quality. Inspect the
outputs before publishing. Commands include process startup and video encoding;
preparation, setup and the warmup command are recorded separately.

## Million birds

[The bird benchmark](birds/README.md) ships all four shot templates and the
corresponding compute programs: original birds2, then skim, dive and reveal. It
runs complete vork movies and hands actual baked flock checkpoints to Blender.
Its Blender adapters remain under `integrations/blender/`. Blender sparse-frame
projections and excluded simulation cost are explicit. Current draft and final
numbers are in [RESULTS.md](RESULTS.md).
