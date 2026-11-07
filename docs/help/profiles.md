PROFILES
Choose with --profile; profile settings are separate from scene content.
std/previs: fast iteration on framing, materials and direct lighting. It omits
indirect lighting; raster surfaces use central-emitter hard shadows. Broad-light
highlights can narrow or change brightness; ray-camera effects can remain grainy.
Judge the finished lighting with final or path.
std/draft: 4 samples with scene-ray indirect lighting and OIDN denoising.
std/final: 8 samples, the default; scene-ray lighting and OIDN denoising.
std/path: 4096 samples; accuracy, especially interiors, indirect lighting, glass and caustics.
Draft, final and path denoise with bundled OIDN by default. --set denoise=false uses the profile
reconstruction (raw path samples for path); --set seed=42 makes sampling repeatable on the same backend.
--set samples=N overrides the profile budget (1..4096); more samples means cleaner and slower.
Without a fast GPU denoiser, draft and final announce it and use 16 and 32 undenoised samples;
--set denoise=true uses the slow CPU denoiser anyway.
See vork help limits for lighting approximations and scene-size limits.
<!-- all -->
PROFILE FILE REFERENCE
A profile file is {"version":1,"level":1,"settings":{"seed":0,"denoise":false}}.
Levels: 0 = std/draft, 1 = std/final, 2 = std/path, 3 = std/previs.
Camera optics and render.motion_blur belong to the scene.
