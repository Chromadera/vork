OUTPUT ENCODINGS
.png one still (t=0 unless --at); other channels beside it as NAME.<channel>.png
PNG filename frame patterns (two or more # characters or printf-style %d/%04d) are usage errors; use a directory or %06d.exr for frames. Parent directories and other filename characters are literal.
directory (no extension or trailing /): NNNNNN.png frames plus channel PNGs
PATH/%06d.exr: one multilayer OpenEXR per frame, scene-linear premultiplied, before exposure
.mp4 H.264; .mov ProRes 4444 RGBA (keeps alpha; transparency needs render.background "transparent")
--at SECONDS: one still; needs .png, directory or EXR. --frame-start/--frame-end N: inclusive
absolute frame indices, default the whole clip; not with --at.
Sidecars: OUT.meta.json for each --out OUT, e.g. check.png.meta.json (fps, per-frame camera and depth unit, object_ids pass_index -> node,
exposure); OUT.camera.json when camera is a channel (videos always; vork help camera-json).
PNG channels: depth and object_index grayscale16 (depth metres per unit in meta.json
frames[].depth_unit_m); view_normal RGB8 = (v+1)/2; motion RGB16, RG = 0.5+0.5*offset in image
extents, clamped to ±1; shadow_catcher RGB16 linear, clamped 0..1; aov linear 16-bit, clamped 0..1.
EXR channels: R,G,B,A; Z (metres, background 1e10); view_normal.X/Y/Z; object_index.X;
shadow_catcher.R/G/B (unclamped multiplicative light ratio); motion.X/Y (pixels, +Y down, current minus previous); aov <name>.X or <name>.R/G/B; float32.
depth: view-axis metres to the nearest surface, not blended across edges.
--exr-codec none|rle|zips|zip|piz|pxr24|b44|b44a|dwaa|dwab (default zip; pxr24/dwa are lossy).
--diagnostics also writes NAME.timings.json.

shadow_catcher follows Cycles: catcher material light with CG / without CG, including
shadows, bounce and reflections. A zero baseline component is neutral 1 without
division; light added by CG to an otherwise unlit catcher is not representable.
Combined always treats a catcher as transparent black on transparent film; opaque
film shows background times ratio. Holdouts are transparent black on transparent
film, opaque black on opaque film. Channel selection never changes combined.
