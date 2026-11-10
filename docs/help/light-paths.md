# Light paths

Custom cameras and per-contribution sensors. Full reference: light-paths --all.
<!-- all -->

A camera material exports `CameraRay camera(CameraContext, Inputs)` and optionally
`float3 sensor(SensorContext, Inputs)`. Inputs use ordinary material tracks.
`std/camera` supplies Blender optics and projective acceleration; uncalibrated
programs trace beauty, depth, normals and IDs from their rays and export no K or distortion model; motion export requires projective calibration.

`CameraContext`: film_size (pixels), filtered uv, lens_sample ([0,1)²), time,
world origin/right/up/forward, focus_distance (object depth or -1).
`uv = (pixel + 0.5 + filter_offset) / film_size` uses zero-based pixel coordinates, normalized to the film extent with (0,0) at the top-left corner and (1,1) at the bottom-right; x increases right and y down, `film_size` is (width,height) in pixels, and per-sample pixel-filter offsets can place `uv` outside [0,1].
`CameraRay`: world origin/direction, weight (RGB; default 1), wavelength_scale
(default 1), t_min/t_max (metres along normalized direction; geometry visibility).
Lens samples are independent per path; weight includes any radiometric Jacobian.

Transport uses scene wavelength = detector wavelength × `wavelength_scale`.
The density convention is radiance per wavelength: a relativistic camera with
Doppler factor D returns wavelength scale D and radiometric weight D⁵. Camera
motion itself is authored with ordinary node tracks. Scene geometry is sampled at
scene time; this is not retarded-time evaluation of animated geometry.

`SensorContext` supplies `path_length` in optical metres, detector `wavelength` in
nanometres, filtered `uv`, and scene `time`. The sensor returns a multiplier in
linear Rec.709. Each arriving contribution is converted from its sampled spectrum
with CIE 1931 matching before this multiplier is applied, then accumulated. Negative
matching lobes are retained. This multiplicative hook is not a replacement colour
matching basis.

Finite emitters emit their impulse at optical length zero. Length includes the
camera-to-near-plane leg and sums phase index × segment length; numerical ray
continuation offsets add no length. Infinite emitters use the largest finite f32
as a non-finite-arrival sentinel. A half-open gate accepts `start <= path_length &&
path_length < end`. Gate inputs animate through ordinary material tracks; no
render gate, speed, spectral switch or wavelength-count setting exists.

Dispersion follows OpenPBR Surface 1.1 `transmission_dispersion_scale` and
`transmission_dispersion_abbe_number`. Its wavelength-dependent index changes
transport, independently of sensor response. Dispersive interfaces retain their
authored roughness on indirect paths so glossy regularization does not erase the
wavelength-dependent refraction. Spectral transport is selected only
when reachable material/sensor outputs use wavelength or dispersion, or the
camera wavelength mapping cannot be proven one. Ordinary RGB paths report
wavelength zero. Surface and emitter contexts use scene wavelength;
sensors use detector wavelength.

Phase optical length is not group delay in a dispersive medium. RGB upsampling
uses positive smooth RGB basis functions and a white-normalized 6504 K Planck
emission illuminant. White reconstructs to white; saturated RGB primaries do not
reconstruct exactly. Extrapolated ultraviolet/infrared content is not uniquely
determined by RGB colours. The detector samples 380–780 nm; a camera mapping can
query scene spectra outside that interval.
