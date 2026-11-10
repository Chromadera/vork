<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Blender import helpers

This directory is a separate GPL-3.0-or-later Blender component; the boundary
with vork is scene, geometry, shader and image files. The renderer and std do
not import bpy or this component.

The 0.2/0.3 releases do not support participating media. For those releases,
use surface-only scenes: disconnect World and material Volume inputs before
exporting. This removes the volume effect; selecting another profile does not
restore it. The bridge still emits volume programs those releases cannot use; see
[fog and volumes](#fog-and-volumes) for the limited 0.4 support and required scene rewrites.

Export a scene headlessly:

```sh
blender -b --disable-autoexec source.blend --python integrations/blender/export.py -- --out /tmp/vork-scene
```

Add `--animation` for scene-sampled clips and camera/light controls. The default
preserves evaluated modifiers and reports any missing morph targets;
`--no-apply-modifiers` retains native shape keys and reports omitted modifiers.
Subdivision is evaluated by Blender at export: Blender converts its 0..1 edge
crease weights to OpenSubdiv sharpness as `10 * weight * weight`. The glTF
bridge exports that evaluated surface, so no Blender crease weights enter the
scene’s OpenSubdiv/USD `crease_edge` sharpness field. `export-report.json` records each
material variant and substitution, missing geometry/transport features and baked
fields. `shot.json` holds the scene and `shot.profile.json` its render profile: level 1
for EEVEE or level 2 for Cycles, plus seed and denoise. Source sample counts are
recorded in the export report; the selected vork profile owns its sample budget. Render with
`vork render shot.json --profile shot.profile.json`. No EEVEE rendering runs in export.

`--bake-curves` samples RGB curve transfer functions into lookup textures to
reduce shader compilation on large material collections. Upstream values and
coordinates remain dynamic, and outside-domain extrapolation stays analytic.
Each bake reports its domain, source frame and image encoding: per-channel
range-encoded PNG8 with 8-bit storage quantization. These one-row textures use
the renderer’s uncompressed path. Native curve lowering
remains the default. The API option is `MaterialWriter(..., best_effort=True,
curve_lookups=True)`.

Use `writer.write_asset(glb_path)` to bind material programs per primitive UV
signature. A shared source material on meshes with and without UVs gets separate
file-level variants; missing source UVs use a reported zero coordinate on that
variant. Geometry bytes and source material slots are preserved.

`world.translate(world)` runs inside Blender 5.2.1 and returns `WorldImport`:
`image` (a datablock or `None`), `environment` (a v2 light node), `background`
and `materials`. The caller writes image scene-linear pixels to HDR bound as
`images.world`, merges `materials` into the scene, and uses `background` in
`render.background`. `export.py` is one caller; the helper has no benchmark or
renderer dependency.

Translation follows the active EEVEE World output, not node insertion order.
Supported graphs contain Background with constant strength; constant RGB, native
EEVEE Blackbody colour (constant Temperature at/above the RNA evaluator’s 800 K
hard limit), or an
equirectangular Environment Texture with linear filtering; Generated coordinates
through Mapping rotations (including Texture inverse rotation and positive
uniform scale); and binary Mix/MixRGB branches driven by Is Camera Ray. Mapping
lookup rotations become inverse light transforms after Z-up to Y-up conversion.
An authored constant camera branch stays separate from World illumination.

### Fog and volumes

Vork 0.4 stage 1 admits finite homogeneous fog in closed convex carriers and
constant or exponential-height world fog through complete-camera path transport,
including shadows and independent fog overlapping OpenPBR glass. See
[`vork help fog`](../../docs/help/fog.md) and the complete
[`examples/fog.json`](../../examples/fog.json). This requires 0.4;
0.2/0.3 binaries do not gain it by selecting a different profile.

The bridge's existing World Volume Scatter lowering still emits the removed
`std/fog` program. Rewrite that output to an authored `medium` program, with physical
RGB `scattering = density * colour`, zero `absorption` and the original
anisotropy strictly between -1 and 1. Attach it as `medium` on the environment
light. This is an explicit scene rewrite, not a compatibility alias. For height
haze multiply absorption and scattering by `exp(-falloff * (ctx.world_position.y - base_height))`, with coefficients at `base_height` and inverse-metre `falloff`; preserve zero channels explicitly when the exponential overflows. See the authored program in `examples/fog-height.slang`. Do not substitute height fog for arbitrary linked density graphs.

Volume-only Scatter/Emission translation uses numeric `blender.volume` helper
functions. Only homogeneous coefficients with bounded callback work and closed
convex carriers (at most eight, each at most 32 triangles) fit stage 1. Combined
surface/volume programs, subsurface surfaces (including animated/textured
subsurface activation), general procedural density, grids and arbitrary
Principled Volume graphs remain unsupported. Use a separate constant fog carrier
overlapping `std/openpbr` glass with subsurface_weight zero, or render unsupported volumes externally and
composite an image plate. `std/fog` and `blender/principled_volume` remain removed.
Importing `blender.volume` alone is not an admission guarantee. The bridge's
translation paths have not been newly qualified against Blender by this change.

`UnsupportedWorld` identifies the node/socket when the graph cannot be represented.
Procedural colours, fractional mixtures, linked mapping parameters/strength,
nonuniform/translated coordinate maps, other/linked volume graphs, groups, and distinct
textured camera/lighting backgrounds need further graph translation or renderer
support. Unsupported nodes are never silently replaced with a different node's
image or constant. Source graphs and materials are not modified.



`material_export.MaterialWriter(directory)` lowers selected Blender surface graphs
into ordinary Slang programs and image inputs. Each program also takes its surface's
representative colour as inputs that it reads: `base_color` (the constant, or the image
read through the active UV, feeding Base Color of the heaviest Principled or Diffuse
BSDF, else Glossy, followed back through colour adjustments and mixes) and `roughness`
(a constant), and its emission as `emission_color` and `emission_luminance` (from the
heaviest Emission shader or emitting Principled BSDF).
`writer.write(material)` returns
an asset material override ID; merge `writer.images`, `writer.programs` and
`writer.materials` into those v2 scene sections. Export geometry with
`geometry.export_gltf(**options)`, using the usual Blender glTF operator arguments.
`geometry.filter_material_overrides(filepath, bindings)` filters requested overrides
against the emitted glTF/GLB material table before lowering graphs. It warns with
the asset path and each missing slot name when Blender drops a slot or its faces;
retained overrides apply normally. Authored scene bindings remain strictly validated.
The helper preserves native TEXCOORD streams and adds stable active/named UV
aliases referencing the same accessors, without copying geometry buffers. It
restores temporary source metadata even if export fails. Packed PNG/JPEG bytes, colour spaces and supported sampler settings
are preserved without applying Blender's display transform.

This subset handles Object/Normal, active UV and named UV Map coordinates, constant affine
Mapping, flat/box images with mapped pixel footprints, RGB Curves, RGBA Mix,
Bump, Invert/HSV, Color Ramp, Layer Weight Facing, Principled, Emission
and Add/Mix Shader, plus Glossy in best effort mode. The writer selects the current source render engine; direct
Compiler callers can pass `engine="CYCLES"` or `engine="EEVEE"`. It requires no fabricated UVs.
Source RGB curves are evaluated through Blender's API and resampled until probes
at knots and interval quarters agree within a 16-bit unit-range channel value;
unrepresentable curves fail. Compact piecewise-linear segments are stored exactly
in dense material buffers and selected by binary search, with the same extrapolation
and lerp arithmetic. One- and two-segment curves remain expressions. Height callbacks reevaluate the complete graph at
neighbouring pixel offsets. Implicit image vectors use the active render UV. UV graphs read typed attributes
with gradients and retain Blender UV orientation through Mapping/Bump. Missing
source UV streams fail installation instead of receiving invented coordinates.
In strict mode, unsupported nodes and instancer coordinates,
image Color with linked Alpha, linked mapping parameters, subsurface/anisotropic/thin-film
responses, the reflection substitutions below and material volume/displacement raise `UnsupportedMaterial` with the
source material/node/socket. Nonzero unsupported responses are never dropped.
Image Texture Alpha samples the stored Alpha channel independently of Color.
The emitted Principled response uses vork's OpenPBR shading; it does not promise
identical Blender BSDFs. No source datablock is edited.



`display.translate(scene)` writes `render.display` with `std/standard`, `std/agx`
or `blender/filmic`; exposure, gamma, dither intensity and applicable looks are inputs.
White balance is emitted inside the display wrapper using `blender.white_balance`;
scene-linear outputs retain their original values.
Non-sRGB display devices, other views (including ACES 1.3), unsupported display curves
and mismatched looks raise named errors. This does not translate compositor
node trees. Supported evaluated RGB display curves are emitted into the display wrapper.


## Licence boundary

The Blender integration is GPL-3.0-or-later ([LICENSE](LICENSE)). It exchanges scene files and CLI invocations with vork. It is published in this repository as a separate component and is not part of vork's release archives or npm/PyPI packages; no renderer, SDK or library code imports it. This marking records the integration’s existing licence boundary; it does not relicense the renderer.

## Partial scene ingestion

For scene ingestion, `MaterialWriter(directory, best_effort=True)` enables partial
surface translation. Failed numeric producers use the destination socket's
source default; failed shader closures use that node's Color/Base Color or the
material viewport colour as a diffuse surface. Other shader branches remain
translated. Each returned material ID has a `writer.reports[id]` record with its
status and material/node/socket/reason/replacement substitutions. Persist these
records with the scene; a partial status does not certify visual fidelity.
Linked displacement and volume on surface materials are reported and omitted.
The volume-only Emission and Volume Scatter lowering produces numeric
medium programs subject to the [fog and volume limitations](#fog-and-volumes). Other volume-only graphs receive a transparent
surface with an explicit
missing transport report so they do not become opaque black occluders.
Strict translation remains the default. No source graph is modified.



Native lowering also supports EEVEE Diffuse BSDF, Glossy (below),
neutral Transparent coverage, instance-bound node groups, reported Generated-coordinate approximation,
Geometry Position/Normal/True Normal/Tangent/Incoming/Backfacing, Fresnel, Layer
Weight Fresnel, Blackbody, scalar/vector Mix and the admitted Math operations in
`blender/nodes`. Cycles rough Diffuse retains a named approximation in best effort mode. Glass BSDF uses native GGX reflection/refraction; Cycles Beckmann and Multiscatter GGX are reported as
single-scatter GGX approximations in best effort. Active-render-UV tangent Normal
Map uses the imported tangent frame; named UV and other spaces remain reported
fallbacks. Unsupported Principled responses preserve the other translated sockets.

vork's material is OpenPBR only, so three Blender reflection choices become their
nearest OpenPBR equivalent. Each is a reported substitution in best effort mode and
raises `UnsupportedMaterial` in strict mode:

- Glossy's constant-colour reflection becomes OpenPBR dielectric Fresnel. The IOR
  makes normal-incidence reflectance equal the colour's brightest channel
  (`specular_weight` carries the rest of an HDR colour) and `specular_color` carries
  its hue, so head-on reflections match and grazing reflections are brighter than
  Blender's: they rise toward the colour's hue at full brightness (a grey Glossy
  rises toward white). On a gold GGX Glossy sphere the mean colour moved by under 0.002.
- Cycles Ashikhmin–Shirley, Beckmann and Multiscatter GGX Glossy become GGX of the
  same roughness. Ashikhmin–Shirley highlights come out sharper and brighter than
  Cycles' at equal roughness (about 18% brighter sphere mean at roughness 0.3).
- Cycles single-scatter GGX Principled metals become multiscatter-compensated GGX
  in the path integrator: rough metals keep the energy single scattering loses, so
  they render slightly brighter than Cycles' GGX (about 1% at roughness 0.5), as
  Cycles' Multiscatter GGX does.

`export.py --bake-values RESOLUTION` enables selective CPU value baking during scene
export. Resolution is explicit; the CLI requires applied render modifiers. Shared
glTF meshes receive separate material tables for each identified source object,
while accessor/geometry bytes stay shared. Ambiguous or repeated source object
occurrences retain reported socket defaults instead of sharing a world-field bake.
Animated exports report that these textures are source-frame snapshots. Bakes
require finite, nondegenerate UV triangles inside one image tile, with no
overlapping interiors; shared edges are allowed. Unsupported UV layouts and
missing source images retain reported defaults. Receipts include the UV layer
and source bake margin. No automatic unwrap changes the authored coordinates.

`writer.write(material, bake_object=obj, bake_resolution=resolution)` adds selective
CPU emission baking when best effort cannot lower a numeric producer. Resolution
is explicit. These materials and textures belong to that evaluated object, UV
layout and frame; bind them per object and export fresh bakes for changed poses.
Bakes bind float EXR images directly. Unmodified EXR/HDR source bytes are retained;
dirty/generated float images are saved as linear EXR snapshots. The renderer uses
filterable RGBA16F with float mips, preserving negative/HDR values (magnitudes
clamp at65504), without PNG8 quantization or block compression. Optional curve
lookup baking still uses its explicitly reported PNG8 encoding.
`writer.reports[id].bakes` records provenance.
View/ray-dependent fields and unavailable source context remain reported defaults.
Baking retains closures and cannot reconstruct hair or volume transport.

`animation.export_clip(path, asset_name, apply_modifiers=...)` exports scene-sampled
glTF animation and returns clip layers plus emitted channel paths/skin counts and
omission findings. Apply Modifiers preserves evaluated geometry and may remove shape keys;
missing emitted morph targets are reported per object. Disabling it retains morph
targets and reports omitted non-armature modifiers. `animation.sample_controls(scene, bindings)` samples native
camera/light transforms and numeric controls; bindings map explicit vork node names
to source objects. Merge its returned layer and the clip layers into scene animation.
Source frame/subframe are restored. Frame samples use linear interpolation; camera
cuts use STEP when every source camera has a binding. Projection/sensor-fit/DOF-enable
changes are reported as initial-configuration snapshots. Orthographic DOF and
focus planes at or inside focal length are reported and omitted. Apply
`sample_controls`’s returned `disabled_dof` names by removing DOF from those
camera nodes; these clips cannot retain thin-lens DOF for their full range.



Render settings and source selection are restored after geometry and control
sampling. Static and animated area lights retain object scale. Light colour is exported as
`color`, and `use_temperature` as `temperature` (kelvin; vork multiplies by std.color's
blackbody, a Planck integration without Blender's constant colour above 12000 K); energy includes
`2 ** exposure`. Unnormalized lights retain a reported normalized-energy
approximation. Shadow technique and map resolution are render-profile choices and are never exported per light. Disabled source shadows become `shadowed_by: []`. Light type or
normalization changes retain initial numeric data across the clip; shape and
shadow toggles retain their initial configuration and report it. Animated material
values remain source-frame snapshots and are explicitly reported.

Volume-only Emission and Volume Scatter translations export numeric
medium programs, leaving no opaque fallback shell. Stage 1 admission still requires
the homogeneous coefficients, convex carriers and explicit scene rewrites described
in [fog and volumes](#fog-and-volumes). Surface-dependent fields remain
reported defaults because medium inputs have no surface UV/frame context.
Unsupported scattering phases use a reported Henyey-Greenstein approximation.
In best effort mode, Light Path uses reported primary-camera values; shadow and
secondary-ray contexts are approximated. This is a named fidelity boundary.



Generated coordinates in best effort use normalized imported rest bounds; source
ORCO/manual texture-space data is not carried by glTF. Strict mode rejects that
missing source context. Object and UV coordinates retain their explicit contracts.

Standard-tone display curves export as a compositor before white balance,
exposure and OCIO. Alpha and control outputs are preserved. Every export reports
that curves also affect linear EXR/render-image beauty; strict translation refuses
that change. Film-like tone and vertical endpoint extrapolation are reported
omissions.



In best effort, Principled Subsurface Weight, Base Color, Subsurface Scale and
Subsurface Radius can drive native opaque Burley diffusion. The report identifies
the source scattering model and the native approximation. Cycles anisotropy uses
native transport similarity; EEVEE keeps its isotropic diffusion response. Strict
translation rejects this approximation. The complete active shader graph must be
proven opaque, nontransmitting and thick: Mix/Add Shader, reroutes, muted bypasses
and group instances participate in that proof. Unsupported or dynamic transport
fields keep the other Principled responses and report omitted subsurface.
Bounded media also omit diffusion with a report while retaining the medium.

Denoising settings map to guides, prefilter and quality.

The comparison bridge lowers constant/evaluable Cycles light emission graphs,
including Blackbody, scalar Math/Map Range and shader mixtures. Ray-dependent
factors that cannot be represented by vork light controls produce a prominent
`BRIDGE SUBSTITUTION` on stderr and a named `export-report.json` omission;
the whole unsupported graph falls back to datablock energy, exposure, colour
and temperature. A linked factor never uses its unlinked socket default: that
proxy overlit Monster and measured worse than the datablock fallback. Constant
light graphs multiply datablock power/colour, as in Cycles.



Mixed directional Worlds can be evaluated by Cycles into a 2048×1024 float32
scene-linear environment map during export. This needs a configured Cycles
device. The bake isolates its scene and leaves the source settings intact;
`world_bakes` records the finite directional resampling. Observer-dependent
coordinates and unrepresentable ray contexts are rejected with a named
substitution, including the replacement lighting/background contract.

Object camera, shadow, diffuse, glossy and transmission visibility is attached
to exact exported glTF node indices through asset overrides. Volume-scatter
visibility has no independent vork equivalent: every nondefault source value
is reported explicitly, while the five supported controls remain independent.

View curves and white balance belong to the generated display program together
with AgX/Filmic/Standard, exposure and gamma. PNG/video use that program; EXR and
float outputs bypass it. Exported display LUTs are bound explicitly. Unsupported
curve shapes remain reported substitutions rather than scene-linear compositors.

On transparent film, the display ABI supplies unassociated RGB without alpha.
Curves therefore cannot reproduce Blender's premultiplied-curve RGB for partial
coverage; export records that display-only substitution explicitly. Alpha and
raw scene-linear outputs remain intact.
