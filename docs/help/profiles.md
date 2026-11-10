PROFILES
Choose with --profile; profile settings are separate from scene content.

Profile     Speed       Use                         Main omissions
std/previs  fastest     framing and direct light    no indirect bounce lighting
std/draft   quick       lighting previews           approximate GI, 4 samples
std/final   moderate    delivery (default)          approximate GI, 8 samples
std/path    slowest     interiors, glass, caustics  finite depth/budget, 4096 samples

Preview means std/previs; use the names above with --profile.
For a quicker path check, start with --profile std/path --set samples=64;
raise samples for noisy shadows, glass and caustics. Resolution alone does not
reduce the per-pixel sample budget. More samples cost more time.

Design policy: [profiles as execution strategies](../design/profile-strategies.md),
including the real-time rule for new primitives and preparation before playback.
A real-time profile is future work.
std/previs: fast iteration on framing, materials and direct lighting. It omits
indirect lighting; raster surfaces use central-emitter hard shadows. Broad-light
highlights can narrow or change brightness; ray-camera effects can remain grainy.
Judge the finished lighting with final or path.
Hair uses complete-camera path transport automatically, including render-image
captures. Promoted views keep the sample budget and default to denoising;
--set denoise=false turns it off. Glass and water retain final’s raster transport.
std/draft: 4 samples with scene-ray indirect lighting and OIDN denoising.
std/final: 8 samples, the default; scene-ray lighting and OIDN denoising.
Final's approximate GI can omit bounced paths such as camera → diffuse → glossy → light.
A custom surface material selects the older one-hit GI for the whole scene.
More samples cannot recover omitted paths; choose --profile std/path for full bounce lighting.
std/path: 4096 samples; accuracy, especially interiors, indirect lighting, glass and caustics.
Draft, final and path denoise with bundled OIDN by default. --set denoise=false uses the profile
reconstruction (raw path samples for path); --set seed=42 makes sampling repeatable on the same backend.
--set samples=N overrides the profile budget (1..4096); more samples means cleaner and slower.
Small text, thin geometry and subpixel bevel highlights can need extra camera samples;
see vork help geometry for the spatial sampling limits.
Without a fast GPU denoiser, draft and final announce it and use 16 and 32 undenoised samples;
--set denoise=true uses the slow CPU denoiser anyway.
See vork help limits for lighting approximations and scene-size limits.
<!-- all -->
PROFILE FILE REFERENCE
A profile file is {"version":1,"level":1,"settings":{"seed":0,"denoise":false}}.
Levels: 0 = std/draft, 1 = std/final, 2 = std/path, 3 = std/previs.
Camera optics and render.motion_blur belong to the scene.
