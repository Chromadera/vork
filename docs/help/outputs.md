OUTPUT ENCODINGS
.png RGBA still (keeps alpha; use render.background "transparent"); t=0 unless --at; other channels beside it as NAME.<channel>.png
PNG filename frame patterns (two or more # characters or printf-style %d/%04d) are usage errors; use a directory or %06d.exr for frames. Parent directories and other filename characters are literal.
directory (no extension or trailing /): NNNNNN.png frames plus channel PNGs
PATH/%06d.exr: one multilayer OpenEXR per frame, scene-linear premultiplied, before exposure
.mp4 H.264 (requires opaque final alpha on every composited frame); .mov ProRes 4444 RGBA (keeps final alpha)
--at SECONDS: one still; needs .png, directory or EXR. EXR stills also require %06d
in the filename, even with --at; a plain still.exr is refused.
--frame-start/--frame-end N: inclusive
absolute frame indices, default the whole clip; not with --at.
Sidecars: OUT/meta.json for directories; OUT.meta.json for files (fps, per-frame camera and depth unit, object_ids pass_index -> node,
exposure, warnings [{code,requested,dropped}] for omitted/substituted effects); camera.json in directories or OUT.camera.json for files when camera is a channel (videos always; vork help camera-json).
EXR sidecar names remove %06d and trailing ._- from its filename prefix, keeping
the extension. A filename of %06d.exr uses directory sidecars in its parent.
The same naming rule applies to camera.json and timings.json.
PNG channels: depth and object_index grayscale16 (depth metres per unit in meta.json
frames[].depth_unit_m); view_normal RGB8 = (v+1)/2; motion RGB16, RG = 0.5+0.5*offset in image
extents, clamped to ±1; shadow_catcher RGB16 linear, clamped 0..1; aov linear 16-bit, clamped 0..1.
EXR channels: R,G,B,A; Z (metres, background 1e10); view_normal.X/Y/Z; object_index.X;
shadow_catcher.R/G/B (unclamped multiplicative light ratio); motion.X/Y (pixels, +Y down, current minus previous); aov <name>.X or <name>.R/G/B; float32.
depth: view-axis metres to the nearest surface, not blended across edges.
--exr-codec none|rle|zips|zip|piz|pxr24|b44|b44a|dwaa|dwab (default zip; pxr24/dwa are lossy).
--diagnostics: OUT/timings.json for directories; OUT.timings.json for files.

shadow_catcher follows Cycles: catcher material light with CG / without CG, including
shadows, bounce and reflections. A zero baseline component is neutral 1 without
division; light added by CG to an otherwise unlit catcher is not representable.
Combined always treats a catcher as transparent black on transparent film; opaque
film shows background times ratio. Holdouts are transparent black on transparent
film, opaque black on opaque film. Channel selection never changes combined.

meta.json is version 3: frame times are numeric evaluation seconds; fps is a
number or rational string. camera.json remains version 2.
Before writing, a run invalidates previous completion metadata, camera and timing
sidecars. Explicit frame ranges and --at stills preserve numbered PNG/EXR frames
outside the selection; selected frames and their old channels are removed, even
if rendering then fails. Whole-clip renders remove all owned old frames/channels,
including trailing frames. Unrelated files survive. Metadata and sidecars describe
only the current invocation, not accumulated frames. Serialize writers per
destination (concurrent writes fail "output already in use").
Only consume a shot after success with fresh meta.json; partial files can remain
on failure. Cleanup also matches native non-UTF-8 filenames.

--png-depth 8|16: PNG colour bits per component; default 8. Requires PNG colour
output for 16. Both depths use the same exposure and display transform. Standard
display dither scales to the output bit depth; control channels retain their depths.
