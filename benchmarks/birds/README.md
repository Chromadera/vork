# Million-bird comparisons

**Draft and final measurements are recorded in [RESULTS.md](../RESULTS.md).** Preview and path timings there are explicitly carried forward.

The forkable inputs cover the
original `birds2` film and the later `skim`, `dive` and `reveal` cameras. Each has
one million simulated birds and nine authored foreground birds. The templates and
compute programs retain the measured shot's content; wing phases are generated
as binary float32 instead of a million inline JSON numbers. Downloaded assets
come from the hash-pinned [murmuration preparation recipe](../../examples/murmuration/).
Keep its [credits](../../examples/murmuration/CREDITS) with renders.

From the checkout root, on a real supported GPU with vork, Blender 5.2.1 and ffmpeg:

```sh
uv run --with numpy --with blake3 python benchmarks/birds/run.py --shot birds2 --out /tmp/birds2-bench
uv run --with numpy --with blake3 python benchmarks/birds/run.py --shot skim --out /tmp/skim-bench
python3 benchmarks/birds/summarize.py /tmp/skim-bench
```

Use `--vork /path/to/vork`, `--blender /path/to/blender`, and `--repetitions 5` as
needed. `--shot dive` and `--shot reveal` run the other later cameras. Results,
images, video, simulation checkpoints, exact commands, exit codes, wall times,
version and scene hashes stay in the output folder. Asset downloads and setup
are timed separately. Each profile gets a separate warmup and full measured movie.
The current shipped `std/previs` and `std/final` profiles are used; `std/draft` remains available, with four samples and OIDN denoising.

The Blender adapters live in [the separately licensed integration](../../integrations/blender/).
They bake actual simulation checkpoints before timing and hand the completed flock
to Blender as geometry-node instances. Blender is charged for updating and drawing
that state, not solving the flock. The reader checks checkpoint integrity and
size and fails if the renderer's internal cache format changes. This is a benchmark
reader, not a supported renderer API. Blender's procedural vegetation, analytic
sun disk, bloom, camera roll and authored wing mask are not fully equivalent;
material translation and subframe bird motion are approximations. Compare the
rendered frames yourself; these are timing comparisons, not equal-quality speedups.

Blender renders one first frame and eight distributed warm frames at 16 samples
with EEVEE ray tracing, plus a four-sample preview with ray tracing off. Compare a
four-sample vork movie with the four-sample projection, not the 16-sample one. Cycles is
sampled separately. `summarize.py` labels the projected whole-movie time explicitly.
No sparse frame extrapolation is a measured Blender movie. It excludes scene
construction, process startup, simulation and video encoding, which are different
from vork's complete movie command boundary. `birds2` also includes an EEVEE
four-sample ray-tracing-on row. Setup/load/save and every frame have event receipts.
