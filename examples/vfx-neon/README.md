# After Rain

A suspended cyan seed and an original “AFTER RAIN” blade sign in a wet night
alley. The real camera moves forward; the sculpture's braided ribs occupy
separate depths, while its steel brackets attach to the right-hand wall.
Catcher geometry supplies wall spill and wet-street reflections. The clean
version and glowing-line wipe share the same rendered samples.

The plate is an empty alley after rain: it does **not** demonstrate passing-person
lighting or occlusions, or clearly visible falling rain. It is a moving-camera
compositing example, not a complete demonstration of those effects.

All source footage, generated scenes, measurements and review media stay outside
the checkout. From this directory:

```sh
python3 -m venv /tmp/vfx-neon-tools
/tmp/vfx-neon-tools/bin/pip install -r requirements.txt
PY=/tmp/vfx-neon-tools/bin/python
MEDIA=/absolute/path/outside/checkout/vfx-neon/media
EVIDENCE=/absolute/path/outside/checkout/vfx-neon/validation
$PY prepare.py --media "$MEDIA"
$PY camera.py --media "$MEDIA" --evidence "$EVIDENCE"
$PY author.py --media "$MEDIA"
vork render "$MEDIA/clean.json" --at 2.5 --out /outside/checkout/preview.png
```

Use a fresh evidence directory for a new solve. `camera.py --export-only` reuses
an existing `planar-adjusted.npz`. Reconstruction uses masked static walls and
fence texture, excluding the road, reflections, car, sky and lamps. Dense
forward/backward flow and direct first-frame patch registration retain tracks
by image-correlation criteria. Alternating retained tracks form the fit and
held-out sets; held-out residuals are not culled to improve the reported maximum.
The final fit constrains the right-hand wall to a plane; its iteration cap may
be reached before numerical convergence, so inspect the residuals. Held-out point positions
use the first and last frames; their intermediate observations do not fit the
camera. Endpoints therefore are not independent validation observations.

`contact-validation.json` records every frame's projected and tracked texture,
and four wall-attachment positions compared with a homography fitted only to
held-out texture. Those attachment references are **interpolated virtual points**,
not separately tracked physical bolts. The floor is only a reflection receiver:
there are no ground contacts and no certified ground-slip measurement. The focal
length, scale and off-wall depth are not independently surveyed; planar
self-calibration cannot establish all of them uniquely.

Render on a real GPU at 1920×1080, 24 fps, five seconds. Native 1920×825 framing is
letterboxed without synthetic camera movement. Keep linear clean EXRs for
`reveal.py`; FFmpeg encodes the final sequences. `review.py` checks scene/camera
consistency and exports all encoded frames, contact sheets, lock overlays and
quarter-speed movies. Inspect every frame and the native-resolution edges;
these exports do not automatically certify appearance.



The source is Rafael Fernanz's [“Ally”, Pexels 18972999](https://www.pexels.com/video/ally-18972999/).
`assets.json` records the download, SHA256 and licence-check date. The
[Pexels Licence](https://www.pexels.com/license/) permits modified commercial use
subject to its restrictions. Stock media is not redistributed here. The
sculpture and stroke lettering are original procedural geometry dedicated to CC0.

Lighting is an artistic single-exposure estimate. A graded catcher ratio and a
small positive floor in dark ground pixels make the reflected light visible;
this is not recovered additive HDR radiance. The flat floor does not reconstruct
individual puddle boundaries, and the CG tubes remain sharper than the soft
source plate. Wall-registration statistics do not certify the reflected image.
No renderer defaults are changed.

The empty alley has no pedestrian or umbrella occlusion. The compositor assumes
constant visibility; this shot does not validate moving foreground holdouts.
