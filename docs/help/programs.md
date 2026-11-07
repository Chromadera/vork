PROGRAMS
Use built-in materials first. Custom Slang programs provide surface shading,
displacement, environments, cameras, sensors, compositing and compute.
Use vork help programs --all for the advanced reference and stage declarations.
Procedural textures: import std.procedurals; signatures: vork help std --all.
<!-- all -->
PROGRAM REFERENCE
Slang modules: `import vork;`, one `struct Inputs {...}` whose initializers are
defaults; meshes require surface (lit/displace supplement surface):
  Closure|Blend|Hair surface(ShadingContext, Inputs)
    OpenPBR emission is shared by hits and light samples; lit output is authored shading.
    Light samples use zero differentials; hits retain their shading footprint.
  float3 displace(DisplacementContext, Inputs)       object-local offset after deformation/subdivision; fixed topology
    Use displacement on ordinary meshes with consistent assignments and attributes at shared vertices.
    Hair uses curve geometry and a surface stage only.
  void aov(ShadingContext, Inputs, inout AovOutputs) write_aov(output, inputs.a, value)
  Closure lit(ShadingContext, Inputs, Lighting)      shader-to-RGB: return unlit(rgb)
  float3 environment(EnvironmentContext, Inputs)    radiance
  float4 composite(CompositeContext, Inputs)         vork help compositor
  CameraRay camera(CameraContext, Inputs)           camera.material; vork help light-paths
  float3 sensor(SensorContext, Inputs)               per arriving contribution
  float3 display(DisplayContext, Inputs)
  void compute(ComputeContext, Inputs)               buffers: <binding>_load/_store/_count; dense grids also <binding>_dims (uint3)
Initialized read-only global tables (including static const arrays) are allowed.
Global resources and mutable globals are refused; use Inputs or compute buffers.
Inputs bind numbers, bools and enum tokens directly; Image {"image"}, Aov {"aov"},
FloatGrid/VectorGrid {"grid"}, Attribute<float..float4> {"attribute"}.
[InputRange(min, max, min_closed, max_closed)] on numeric Inputs fields validates
bindings componentwise; booleans select inclusive endpoints. It does not clamp values.
Closure.geometry_normal is a world-space normal.
Closure c = {}; is OpenPBR defaults (fields are OpenPBR input names). blend(a,b,w) weights a by
1-w and b by w; add(a,b) keeps both; mix_closure(a,b,w) interpolates parameters (one lobe).
read_attribute(inputs.a, ctx) returns AttributeSample<T> (.value/.dx/.dy) for
ShadingContext and T directly for DisplacementContext; names and types must match
every bound geometry or instance column.
ShadingContext: time (s), instance_index (authored row), primitive_id, front_facing,
tangent_sign; wavelength (nm; 0 in RGB) is available in surface.
DisplacementContext has only undisplaced object-local POSITION, NORMAL and time (s).
Per-instance displacement values use bound Attribute<T> inputs.
For topology:"points", displace evaluates once per undisplaced object-local point centre and translates the whole sphere without changing its width; NORMAL and generated surface UV/tangent inputs are unavailable.

Noise primitives: import std.procedurals; compose noise/ramps for wood, marble or stone.
Wave and brick patterns: import blender.procedurals; vork help blender --all.
signatures, case-sensitive enums and a surface example: vork help std --all.

SIMULATION
Use scene.simulation with t0, positive dt, seed, state buffer names, and ordered init/steps
program names. Each named program uses schedule:"simulation" and ordinary bindings/dispatch.
S0 is initialized at t0; before t0 hold S0. Render subframes hold the preceding fixed tick.
Tick inputs sample the left endpoint using ordinary numeric animation tracks. Passes have
GPU barriers; use separate generations for neighbour reads/writes. u32 bindings also expose
_atomic_min(index,value), returning the previous value; out-of-bounds returns UINT_MAX.
Seeking replays from initialization or a compatible bounded shared-cache checkpoint.
Keep kernels race-free for reproducible seeking on the same backend. Use computed
placement for opaque, unskinned geometry; author material attributes and transparent placements.

EnvironmentContext: normalized direction, time (seconds), camera (camera escape versus lighting).
Environment programs depend on direction and may sample file images.
Displacement may sample static file images. For points use POSITION, time and bound
attributes; use surface shading without displacement on computed or implicit geometry.

`scene.propagation={program,bounds}` selects `PropagationStep propagation(PropagationContext,Inputs)`; bounds are world-space `[min,max]` metres.
Context: `position` (metres), unit `direction`, `time` (seconds), `previous_distance` (metres; zero at launch). Step: `direction` (normalized), positive `distance` (metres), `absorbed=false`, `radiance=float3(1)` (multiplied at segment start).
Use std/path for authored propagation; use sampled shutter blur rather than motion channels.
Segments use ordinary ray transport; leaving bounds continues straight and samples the sky along the exit direction.

Image inputs bind only `{image: name}`. Programs own channel selection, factors and UV transforms.
`std.texture` provides footprint filtering and tangent-frame normal mapping; `ShadingContext.sample_spread` retains sample-centre covariance, while UV derivatives remain geometric.
