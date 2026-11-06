LIMITS
Supported GPUs: Apple Silicon Metal and recent NVIDIA Vulkan with hardware ray
tracing and storage buffer arrays. AMD, Intel and CPU/software rendering are
unsupported. Run vork setup to check the adapter. Windows x64 ships install.ps1;
macOS and Linux ship install.sh. See README.md (Install) for binary setup.

Participating media (fog, smoke and volumes) are absent from this release's API.
Legacy medium bindings are ignored with a warning; surface subsurface scattering
is supported.

std/final approximates indirect lighting and can darken interiors, including glossy
bounces. More samples do not restore omitted transport. Use std/path for interiors,
glass and caustics. Shipped path transport still has finite bounce budgets,
indirect clamping, adaptive sampling and denoising; it is not an unclamped oracle.
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

vork setup warms the default profile; setup --all warms the bundled catalog.
Authored programs and uncovered layouts compile on first use. Cold compilation
can take minutes; setup does not guarantee instant first renders.

Hair displacement and lit stages are ignored with a warning. Curved propagation
selects path transport automatically; motion channels are zero with a warning,
while sampled shutter blur remains available.

Named opacity/alpha-cutoff attributes on ray-visible OpenPBR materials use program
defaults with a warning. Unsupported custom surface resources fall back to OpenPBR.
Computed geometry with skinning or transparency uses the authored snapshot;
displacement on computed/implicit geometry and normal-dependent point displacement
are ignored. Computed instance material attributes retain authored values.
Legacy spatial environments evaluate at the origin. Extra environment lights and
unsupported light masks are ignored with warnings. Environment programs sampling
render images fall back to the light colour. Inspect warnings and the image.
