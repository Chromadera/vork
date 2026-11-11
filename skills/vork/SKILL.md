---
name: vork
description: Author and render vork scene JSON, stills and animated videos; use std geometry builders and diagnose render refusals and warnings.
---

# vork

## Install and discover

Use the [official installation instructions](https://github.com/voyager-so/vork#install)
for the complete engine, Slang toolchain, libraries and SDKs; a lone copied executable
is insufficient. JSON needs no Python or Node. Apple Silicon uses Metal; recent
ray-tracing NVIDIA GPUs use Vulkan. Linux video needs ffmpeg. Linux software Vulkan
is an explicit, slow fallback: add `--gpu cpu` to setup/render, or set `VORK_GPU=cpu`.

```sh
vork --version
vork setup --skip-warmup
vork help
vork init my-shot
cd my-shot
```

`setup` without `--skip-warmup` also warms the starter's shaders; new programs compile
on first use. Keep stderr: compilation and frame progress can take time.
The generated scene.json renders immediately. Edit it or author plain JSON below.
For SDK authoring, read `vork help sdk`; use the SDK matching the executable,
regenerate JSON after source edits, and keep generated buffer siblings with it.

## Scene shape

Unknown fields are errors. Omit defaults. Units: metres, seconds, radians, linear
Rec.709 RGB; lens/sensor dimensions are mm. Right-handed Y-up; cameras aim local -Z.
Nodes are a tree (`children`); resources are name-keyed maps, referenced by name or
inline. Node variant fields are flat siblings of `type`. Names are globally unique.

This complete scene.json has an animated object, lit ground and camera:

```json
{
  "version": 2,
  "nodes": [
    {"name":"ball","type":"mesh","geometry":{"std":"sphere","radius":1},
     "materials":[{"inputs":{"base_color":[0.12,0.35,0.65],"specular_roughness":0.3}}]},
    {"type":"mesh","translation":[0,-1,0],"geometry":{"std":"grid","size":[10,10]}},
    {"type":"light","shape":"rectangle","translation":[-3,5,4],
     "target":[0,0,0],"energy":700,"size":4,"color":[1,0.92,0.8]},
    {"name":"camera","type":"camera","translation":[3,2,5],"target":[0,0,0]}
  ],
  "animation":[{"tracks":[{"path":["nodes","ball","translation"],
    "keys":[[0,[-1,0,0]],[1.5,[1,0,0]],[3,[-1,0,0]]]}]}],
  "render":{"camera":"camera","resolution":[320,180],"duration":3,"fps":12}
}
```

Materials default to `std/openpbr`; inputs use names such as `base_color`,
`base_metalness`, `specular_roughness`, `transmission_weight`, `emission_luminance`.
A background RGB changes the backdrop, not illumination; add lights.
Mesh `Node.instances` accepts custom attribute columns in `instances.attributes`; bind a column to a material input with `{"attribute": name}` (see `vork help materials` and `vork help nodes --all`).
Camera `target` may be a point or named node. To animate aim, animate a target node;
`target` itself is structural. Rotation accepts a quaternion or `{"euler":[x,y,z]}`.
Animation paths use node names, not array indices; keys are seconds, hold endpoints,
and default to LINEAR for numbers, STEP for strings/bools. Topology stays fixed.

Use `{"std":"NAME",...options}` as geometry inline or in the geometry table:
box, sphere, grid, cylinder, cone, torus, capsule, revolve, rounded_box, extrude, text.
Dimensions and builder options differ: look them up with `vork help std --all`.
`revolve.profile` contains [radius,height] pairs around +Y, with axis endpoints to
close the ends. Text requires an actual TTF/OTF file (`font`), not a family name;
`size` is metres per font em. Font and asset paths resolve beside scene.json.
Standalone geometry builders return `{buffers,geometry}` maps to merge, not a whole scene:

```sh
vork std sphere '{"name":"ball","radius":1}'
```

`sky` returns environment/sun nodes and materials; `environment` preprocesses HDR
lighting into a scene fragment and companion files. See `vork help lights --all`
before using either; merge the returned tables/nodes, keep the generated files.

## Check and render

```sh
vork check scene.json --profile std/previs
vork render scene.json --profile std/previs --out still.png
vork render scene.json --profile std/previs --out clip.mp4
```

For the scene above: one PNG at t=0 and a 3-second, 36-frame MP4.
Use `--at 1.5` for a still at seconds; `--frame-start 0 --frame-end 35` selects
inclusive absolute frame indices. FPS changes output timing and frame count,
not key times. Set `render.duration` in the scene; it is not a `--set` option.

Quality belongs to the render call, never scene content:

| Profile | Use |
| --- | --- |
| std/previs | Framing/direct light; omits indirect light |
| std/draft | Quick approximate GI; 4 samples |
| std/final | Default delivery; approximate GI; 8 samples |
| std/path | Full bounce transport for interiors/glass/caustics; 64 samples |

`--set samples=64` overrides per-pixel budget (1..4096); it cannot restore paths a
profile omits. Draft/final/path normally denoise; `--set denoise=false` disables it.
Without a fast denoiser, fallback budgets may change: inspect installed settings on
stderr and metadata. `--set resolution=640x360`, `--set fps=24`, `--set seed=42`
are call overrides. Check with the intended profile before rendering it.

Output extension selects behavior: `.png` is one still; a directory is a whole PNG
sequence; `frames/%06d.exr` is scene-linear multilayer EXR (also required for an EXR
still). PNG filename patterns are refused. MP4 requires opaque final alpha;
MOV keeps alpha. Set `render.background` to `"transparent"` for transparent PNG/MOV.
PNG/video use the display transform (default AgX); EXR stays scene-linear.

## Refusals, warnings and help

`check` loads assets, expands std, validates programs and animation without a GPU;
it does not prove GPU memory, encoder availability or the resulting picture.
Exit codes: 0 success; 2 usage; 3 scene/program; 4 device/memory/encoder/setup;
5 filesystem. Follow the diagnostic's field/path and suggested fix; do not silently
drop the requested effect. Nonzero can leave partial outputs. Require exit 0 and
fresh `still.png.meta.json` / `clip.mp4.meta.json` (directory: `meta.json`).
Warnings print on stderr and in metadata as `{code,requested,dropped}`: inspect
what was omitted/substituted and tell the user. Inspect the actual image and video.

Start at `vork help`; use `vork help TOPIC --all` for full fields,
`vork help field render.duration` for an exact field. Useful topics: geometry,
std, materials, cameras, lights, animation, profiles, render, outputs, images,
nodes (glTF assets), programs, errors, limits. Asset nodes use
`{"type":"asset","file":"model.glb"}`; keep relative textures/buffers/programs
and licensed fonts with the scene when moving it.

Five source examples to adapt (keep their companion assets; read preparation notes
before rendering downloaded assets): [examples](https://github.com/voyager-so/vork/tree/main/examples)

- `examples/first-scene.json`: std primitives, OpenPBR, area light, camera.
- `examples/product-orbit.json`: camera/object animation and shared resources.
- `examples/programs.json`: authored Slang materials and bindings.
- `examples/compositor-bloom.json`: ordered compositor, intermediate images, white balance.
- `examples/fog-interior.json`: interior lighting, world medium, external geometry buffers.
