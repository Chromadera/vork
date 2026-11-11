RENDER
<!-- generated-call-keys:start -->
--set KEY=VALUE accepts resolution, fps, seed, samples, denoise.
<!-- generated-call-keys:end -->
Values are JSON; resolution also accepts WxH or W,H. --frame-start/--frame-end select inclusive absolute frame indices; --at selects seconds (number or rational string, e.g. --at 1/24).
--gpu cpu (or VORK_GPU=cpu) explicitly selects software Vulkan rendering; see limits.
--gpu N (or VORK_GPU=N) renders on hardware GPU N in PCI bus order. On multi-GPU NVIDIA Linux machines, verify the indices with nvidia-smi.
To use several GPUs, run one render per GPU on separate --frame-start/--frame-end ranges with separate output directories, then join matching MP4s with ffmpeg -f concat -c copy; one vork setup covers identical cards with the same driver.
All destinations share the call fields. transparent keeps environment lighting.
render.display: named or inline display program; default std/agx. EXR stays scene-linear.
std/agx and std/standard follow Blender 5.2.1; exposure, gamma and dither_intensity are inputs.
A white RGB background looks grey by default because AgX maps scene-linear
white into the display's middle range to leave room for bright highlights.
std/standard at default exposure/gamma maps white background RGB to screen white;
this display transform affects the whole image. EXR remains scene-linear.
TONE CURVE AND GRAIN
Set the tone curve on render.display, for example:
"display":{"program":"std/agx","inputs":{"look":"high_contrast","exposure":1}}
inside render. Use look "none" for the default AgX look, or program "std/standard"
for the Standard display transform. `vork help field std/agx` lists the inputs.
There is no built-in film-grain input. Author a display program for artistic grain;
`vork help programs --all` describes DisplayContext (pixel, seed, bit_depth).
Display dither is controlled by dither_intensity (0 disables it); it reduces
encoding banding. For sampling noise, increase --set samples=64 or use
--set denoise=true; these controls do not add artistic grain.

MOVIES
Set render.duration in seconds and render.fps, e.g. "duration":2,"fps":24,
then run `vork render scene.json --out shot.mp4` to render the whole 48-frame clip.
A .png destination renders one still. Use `--out frames/` for a PNG sequence,
`--out shot.mov` to keep alpha, or `--frame-start 0 --frame-end 23` for an
inclusive frame range. MP4 needs opaque final alpha; `vork help outputs` covers encodings.

Motion-blur shutter is in output frames: exposure time in seconds is shutter/fps.
Previous-pose and shutter sample times clamp to zero before animation-layer retiming.
AgX exposure is stops, default 0, with no automatic exposure.
RGB backgrounds affect camera escapes only, not scene illumination.
Named display inputs use material tracks; look/bool keys use STEP. White balance is a compositor.

The exposure summary measures display RGB weighted by alpha. Transparent renders
report clipping, near-black share and mean luma of covered pixels; an entirely
transparent image reports undefined exposure. Opaque black backgrounds count.

Setup separates required checks from optional shader warming. Required stage
errors/timeouts fail; only the default shader-warmup deadline can end successfully
with compilation deferred. Use setup --skip-warmup for checks or setup --all for
unbounded shader warming; completed device-local cache entries survive reruns.

Progress on stderr reports frames delivered and sample passes recorded within a frame.
Recorded passes are CPU preparation. GPU completion counts advance at existing
submission boundaries, so updates can arrive in groups; once all passes
are recorded, the renderer may still be waiting for the GPU, denoising or output.
On Windows, Ctrl+C and Ctrl+Break finish submitted frames, close output files and
print the saved frame count and paths before exiting with code 130. A long GPU
frame or shader compilation must finish before cancellation reaches a safe boundary.
