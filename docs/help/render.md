RENDER
<!-- generated-call-keys:start -->
--set KEY=VALUE accepts resolution, fps, seed, samples, denoise.
<!-- generated-call-keys:end -->
Values are JSON; resolution also accepts WxH or W,H. --frame-start/--frame-end select inclusive absolute frame indices; --at selects seconds (number or rational string, e.g. --at 1/24).
All destinations share the call fields. transparent keeps environment lighting.
render.display: named or inline display program; default std/agx. EXR stays scene-linear.
std/agx and std/standard follow Blender 5.2.1; exposure, gamma and dither_intensity are inputs.
Named display inputs use material tracks; look/bool keys use STEP. White balance is a compositor.

The exposure summary measures display RGB weighted by alpha. Transparent renders
report clipping, near-black share and mean luma of covered pixels; an entirely
transparent image reports undefined exposure. Opaque black backgrounds count.

Setup separates required checks from optional shader warming. Required stage
errors/timeouts fail; only the default shader-warmup deadline can end successfully
with compilation deferred. Use setup --skip-warmup for checks or setup --all for
unbounded shader warming; completed device-local cache entries survive reruns.
