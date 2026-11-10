FOG AND VOLUMES
Author a Slang program with MediumCoefficients medium(MediumContext, Inputs).
Bind its material to light.medium on the single environment light for world fog,
or to a closed triangle mesh for finite homogeneous fog. No surface stage
is needed on a carrier. See vork help programs --all for program bindings.

MediumCoefficients fields all default to zero:
  absorption: float3, nonnegative finite extinction per world metre.
  scattering: float3, nonnegative finite scattering per world metre.
  emission: float3, nonnegative finite scene-linear Rec.709 radiance added per world metre;
    independent of extinction, not surface emission_luminance in cd/m².
  anisotropy: float, finite, strictly (-1,1); Henyey–Greenstein phase,
    positive forward, zero isotropic, negative backward.
Total extinction is absorption + scattering. Values are validated, not clamped.
MediumContext supplies position (carrier-local; world for the environment),
world_position (metres), time (seconds), wavelength (nm; zero in RGB).
Node scaling changes carrier bounds, not coefficient units.

Stage 1 admits spatially constant coefficients on finite carriers; world fog may
also use certified exponential-height absorption/scattering. Height extinction
has the form coefficient * exp(-falloff * (world_position.y - base_height)),
with base_height in metres and falloff in inverse metres; emission stays constant.
Zero channels must stay zero even where the exponential overflows. Nonfinite
positive evaluated coefficients refuse the image. Admission proves the supported
coefficient form and bounded callback work; arbitrary density, resource inputs
(images, grids, geometry), unprovable loops and combined surface/medium programs
are refused. Program Inputs initializers define authored input defaults.

World segments end at mathematical infinity. Positive constant extinction closes
the world; uniform nonemitting fog cannot receive light from infinity.
Height fog can have finite optical depth toward the sky. A zero-extinction
emitting channel at infinity is refused, as is an unrepresentable collision position.
Independent carriers and world fog add, including inside glass; a vacuum carrier
cannot subtract fog. Glass priority selects solid IOR/absorption, not independent fog.

Fog selects complete-camera path transport for the shot and render-image captures,
retaining the selected profile's sample budget. Raster and real-time fog have no
qualified fallback. Builtin OpenPBR surfaces, including glass and subsurface,
may coexist with fog, implicit geometry and curved propagation.
Custom surface callbacks, including those paired with displacement, are not yet supported. Deformed carriers and finite-fog
subtree renders remain unsupported. Normal fog paths use sampled free flights and compensated Russian roulette.
An emergency step watchdog reports `terminated_medium_paths` and warns if it fires.
`discarded_medium_radiance` reports the accumulated absolute-luminance sum erased
before sample averaging; additional energy in the untraced continuation is unknown.
Ordinary scenes should not reach this watchdog.
Exact attachment field: vork help field light.medium.
