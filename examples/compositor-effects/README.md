# Authored compositor looks

Copy this entire directory from `PREFIX/lib/vork/examples/compositor-effects/`
(or `examples/compositor-effects/` in the archive or source checkout) to your
project. Keep `highlight.slang` beside `glare.slang`: glare imports it.

Use a local program reference in a compositor material:

```json
{"program":{"file":"vignette.slang"},"inputs":{"factor":0.5}}
```

Program paths are relative to the scene. Preserve the old material inputs.
`chromatic-dispersion.slang` replaces `post/lens_distortion`: it remaps RGB
channels independently, not geometric distortion. `glare-*.json` are complete,
self-contained scenes showing how to save the original image, run additive glare,
and merge it with the original. Run, for example:

```sh
vork render glare-streaks.json --out glare.png --at 0
```

For custom Slang imports, copy the required modules next to your program and
replace `post.vignette` with `vignette`, `post.glare` with `glare`,
`std.highlight` with `highlight`. For `post.lens_distortion`, copy
`chromatic-dispersion.slang` as `lens_distortion.slang` and use
`import lens_distortion;`. Update qualified module references too.
The authored `bloom.slang` program provides a thresholded image pyramid; its
graph wiring is in `../compositor_graph.py`.
