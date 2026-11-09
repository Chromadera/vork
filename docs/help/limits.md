LIMITS
Supported GPUs: Apple Silicon Metal and recent NVIDIA Vulkan with hardware ray
tracing and storage buffer arrays. AMD, Intel and CPU/software rendering are
unsupported. Run vork setup to check the adapter. Windows x64 ships install.ps1;
macOS and Linux provide install.sh as a separate release asset.
The bundled Slang toolchain targets macOS 26; older macOS is unqualified.
See README.md (Install) for binary setup.

Participating media (fog, haze, smoke and volumes) are unsupported in 0.3.
Fog returns in 0.4 with a design independent of pause/resume.
The removed std/fog, blender/principled_volume and blender.volume programs fail
with unknown-program errors, including unused material definitions. Remove those
definitions and medium-only nodes/bindings; see vork help upgrade.
Used medium bindings and volume topology fail with a named alternative: bake
an image plate and composite it, or render the medium externally. Surface
subsurface scattering is supported. Additional environment lights also fail;
combine their contributions in one environment program or precomposite their maps.

std/final approximates indirect lighting and can omit bounced paths such as
camera → diffuse → glossy → light, with built-in or custom surfaces. A custom
surface material selects the older one-hit GI for the whole scene. More samples
do not restore omitted paths. Choose --profile std/path for full bounce lighting,
interiors, glass and caustics. Shipped path transport still has finite bounce budgets,
indirect clamping, adaptive sampling and denoising; it is not an unclamped oracle.
The indirect clamp limits each contribution's mean absolute RGB to 10, matching
Cycles' brightness measure. It scales all channels uniformly to preserve hue;
direct contributions are unclamped. Bright indirect lighting can still lose energy.
Final's rough glass shadows follow straight connections: roughness does not spread
transmitted shadows, light off-axis receivers, or test scattered-direction blockers.

On Apple Silicon, solid glass and heavy interiors can still abort with exit code 4
and "Impacting Interactivity". A retry can succeed; lower resolution is not a reliable
fix. For thin panes use geometry_thin_walled: true. Try std/path or a supported NVIDIA
GPU for repeated failures; path is slower and is not a universal workaround.
Dense imported transmitting geometry has not been qualified on Metal. Large scenes
with dense transparency can lose the device in final even at small resolutions;
try path and test the full shot before committing to a batch.

Large point scenes and working buffers can exceed a device's per-buffer binding
limit even when total free VRAM is sufficient. Follow the error's resolution/profile
advice or simplify scene buffers. Scene import support does not guarantee rendering
at any size.

vork setup warms only the vork init starter at the default std/final profile. Only
shader warming may stop successfully at its ten-minute limit; required GPU/tool
checks fail on errors or their own stage timeouts.
Use setup --skip-warmup for the GPU/tools report only, or setup --all for the
bundled program/profile catalog and built-in descriptors without the shader time limit. You can interrupt setup
safely and rerun it: completed device-local cache entries are retained.
Other scene kinds, authored programs and uncovered layouts compile once on first use. Cold compilation
can take minutes; setup does not guarantee instant first renders.

Hair displacement and lit stages are ignored with a warning. Curved propagation
selects path transport automatically; motion channels are zero with a warning,
while sampled shutter blur remains available.

Named opacity and alpha-cutoff attributes use the shared camera and ray evaluators.
Computed instance material attributes follow GPU publications, including shutter
samples and seeks. Installation refuses these wholly omitted geometry effects:

- displacement reading computed instance attributes: use static/authored values or
  pre-baked displaced geometry;
- displacement on computed/implicit geometry: compute displaced positions in the
  producer, evolve the scalar field, or bake triangle geometry;
- normal-dependent point displacement: use an authored direction attribute or triangles;
- conflicting displacement materials or inconsistent attributes at shared vertices:
  use one displacement material and continuous attributes, or split the geometry;
- computed geometry with skinning/coverage/transparency, or computed placement of transparent
  instances: use opaque unskinned geometry, bake geometry updates, or author placements.

These errors name the node and available alternative. Approximate effects still warn
and render.
Legacy spatial environments evaluate at the origin. Unsupported light masks
are ignored with warnings. Environment programs sampling
render images fall back to the light colour. Inspect warnings and the image. Each dropped effect prints WARNING: REQUESTED EFFECT DROPPED,
with its code, authored request and omitted/substituted behaviour. Every render
meta.json sidecar and SDK render result returns the same structured warnings array;
warning-and-render success does not mean every requested effect was applied.
