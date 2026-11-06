# Split seed

An original luminous ceramic-and-bronze seed sits on a real glass desk. Its
emissive core lights the table, laptop deck and person through plate-matched
shadow catchers. The seed stands in front of the sleeve, to the left of the
hand's full path. A softly expanded hand matte protects the photographed wrist
boundary from uncertain relief-proxy relighting while geometry supplies occlusion.
The shot holds the untouched plate for 0.8 s,
then an example-owned circular wipe grows from the artifact. `clean.json` omits
the wipe. Both are eight seconds, 1920×1080, 25 fps, default render profile.

Footage: **Tired Woman Sleeping on the Table**, **Ron Lach**, Pexels video
[7983323](https://www.pexels.com/video/tired-woman-sleeping-on-the-table-7983323/).
[Pexels Licence](https://www.pexels.com/license/): free commercial use and
modification. No endorsement by the depicted person is implied. `assets.json`
records the source, download, licence and source hash. Keep all media outside the
checkout. Original procedural geometry is CC0.

External preparation uses full-length OpenCV LK tracks, first-frame patch
refinement and a translating camera constrained by the real desk edge, SAM 2.1 Hiera Small
for separate person and reaching-hand mattes, and Depth Anything V2 Small for
an animated relief proxy. The seed is fixed in world space beside the measured hand path.
The body relief is calibrated to the sleeve resting on the solved desk.
The hand matte protects relighting boundaries; it is not used to disguise an
intersection with the seed.
The model weights are Apache 2.0. These are authoring dependencies only.
Install NumPy, OpenCV, SciPy, torch, upstream `sam2` and `transformers`; use the
SAM revision in `assets.json`. Run model preparation on a CUDA host. Download
its checkpoint outside the checkout:

```sh
python3 examples/vfx-artifact/prepare.py --assets-dir /tmp/vork-artifact --models --checkpoint /path/to/sam2.1_hiera_small.pt
python3 examples/vfx-artifact/render.py /tmp/vork-artifact --out /tmp/shot.mp4
python3 examples/vfx-artifact/render.py /tmp/vork-artifact --variant clean --out /tmp/clean.mp4
```

For a small still after preparation:

```sh
vork render /tmp/vork-artifact/clean.json --profile std/final --at 3 --set resolution=320x180 --out /tmp/seed.png
```

This is the base insertion example. The README's final look adds later relighting,
composite treatment and editorial wipes; those are not all reproduced here.
Real footage + rendered 3D; no generative video.

For existing model results use `--finish` instead of `--models`. To rebuild just
the scene and copy its Slang programs, run `build.py /tmp/vork-artifact`.

The named image graph saves CG beauty, filters the shadow catcher at half
resolution with plate/matte edge guidance, combines at full resolution, adds
quarter-resolution lens scatter, and draws the wipe in `finish.slang`.
Everything beyond the renderer is this example's code. `render.py` encodes the
opaque graph PNGs with FFmpeg: vork's MP4 path rejects the transparent CG
background even when the compositor returns opaque alpha.

Run the quantitative checks after building:

```sh
python3 examples/vfx-artifact/verify_lock.py /tmp/vork-artifact
```

`lock-report.json`, `lock-projections.json`, `lock-features.jpg` and the per-frame
`lock-overlays/` are external evidence. Six full-length features and two of six desk-edge samples are
reserved from pose fitting. Four edge samples span the insertion area and constrain the plane directly;
the rigid keyboard features constrain motion along the untextured edge. The lip
height is calibrated on the first half and held fixed for the second half. The report separates fitting error, held-out reprojection,
frame-to-frame residual change, and contact-footprint disagreement with an
independent camera fit. The latter is uncertainty, not ground-truth contact slip.
It also checks presentation timestamps and measures a conservative artifact
silhouette against every hand matte. Review all output frames and the hand
passes in slow motion before delivering; a low fitting RMS is not acceptance.

The reference area is the fixed keyboard, nearby desk props and the front desk lip.
The edge check measures only slip normal to that edge. Glass reflections
move independently and cannot prove contact lock. The camera includes translation
and keeps a fixed estimated 60 mm lens; table height, room scale and relative-depth
calibration remain estimates. Lens distortion and rolling shutter are not solved.
Do not describe this as a measured sub-pixel tabletop solve. The plate and matte
clocks use their original 25 fps presentation times. CG shutter sampling is disabled
to avoid averaging different silhouette frames; the plate retains its captured blur.

The body is a single-view relief, not a scan. A blurred panorama estimated from
the visible room plus warm practical and cool laptop lights approximates
illumination. Hidden geometry, glass response and reflections
remain approximations; this plate does not provide a measured HDR environment.
