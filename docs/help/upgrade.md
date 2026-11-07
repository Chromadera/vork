UPGRADE
Breaking changes, newest release first. Rewrite scenes/scripts; no aliases.

0.2 (from 0.1)
Before installing
- Keep your 0.1 binary/SDK and source scenes. Installing into the same PREFIX
  replaces 0.1; use a separate PREFIX to keep both, and set VORK_BIN explicitly
  when using SDKs. Keep each binary with its bundled runtime libraries.
- If scripts regenerate scatter, run them once with 0.1 BEFORE upgrading and
  save the written instance columns and all referenced buffers. 0.2 cannot run
  scatter builders; a seed alone cannot reconstruct 0.1 placements.

Scene and script changes
- Scatter builders: find scatter calls/imports or {"std":"scatter",...}.
  Replace with author-generated instance columns on the mesh, e.g.
  "instances":{"attributes":{"TRANSLATION":{"type":"vec3","data":[[0,0,0],[2,0,0]]}}}.
  Keep saved instance buffers for exact old placement; otherwise author the
  intended seeded distribution (a seed alone cannot recover old positions).
  Add equal-length ROTATION (XYZW) and SCALE buffers if used; retain custom
  columns and material bindings. See vork help field node.instances or vork help nodes --all.
- Render argument helpers: find TS renderArguments / Python render_arguments.
  Replace subprocess calls with TS render("shot.json", {out:"shot.png"}) or
  Python render("shot.json", "shot.png"). Import render from the same package.
  If managing the process yourself, write explicit arguments instead:
  ["render","shot.json","--out","shot.png","--set","samples=16"].
  Keep existing render options; see vork help render and vork help sdk.
- Removed compositor programs: find program values post/vignette,
  post/lens_distortion, post/glare in materials/render.compositor.
  Replace each with a local Slang composite program, e.g.
  {"program":"post/vignette","inputs":{"factor":0.5}} becomes
  {"program":{"file":"vignette.slang"},"inputs":{"factor":0.5}}.
  Copy the entire shipped examples/compositor-effects directory into your
  project (installed at PREFIX/lib/vork/examples/compositor-effects). It contains
  vignette.slang, chromatic-dispersion.slang, glare.slang and highlight.slang,
  plus complete glare-*.json merge graphs. Keep highlight.slang beside glare.slang;
  glare imports it. Program paths are relative to the scene; preserve inputs.
  The old lens effect was RGB chromatic dispersion, not geometric distortion.
  Glare produces an additive image: retain the original and merge it afterward,
  as examples/compositor-effects/glare-streaks.json demonstrates. std/bloom is
  still supported for bloom; it is not an exact replacement for these effects.
- Custom Slang imports also break: post.vignette, post.lens_distortion,
  post.glare and std.highlight were removed. Copy the same example modules next
  to your custom program. Use import vignette;, import glare; and import highlight;.
  For post.lens_distortion, copy chromatic-dispersion.slang as lens_distortion.slang
  and use import lens_distortion;. Update any qualified module references too.
- Fog/volume materials: std/fog, blender/principled_volume and blender.volume
  are no longer release programs. 0.1 could ignore legacy medium bindings with
  a warning; 0.2 fails with unknown-program errors. Remove unused definitions
  too. For a scene that relied on ignored fog, remove the medium-only nodes and
  their material bindings/definitions to retain the no-fog result. Preserve any
  surface material on shared geometry. There is no supported volumetric
  replacement in 0.2; keep 0.1 separately if you need its experimental behavior.
- Omitted mesh material: find meshes with omitted or empty materials. Default OpenPBR
  specular_roughness is now 0.3 instead of 0.5. To keep the old look, add
  "materials":[{"program":"std/openpbr","inputs":{"specular_roughness":0.5}}].
  Explicit and imported materials keep their own values.
- TS sky/environment options: find snake_case keys in sky(...) and
  environmentImage(...). Rename color_space -> colorSpace;
  sun_elevation -> sunElevation, sun_rotation -> sunRotation,
  air_density -> airDensity, aerosol_density -> aerosolDensity,
  ozone_density -> ozoneDensity, sun_size -> sunSize,
  sun_intensity -> sunIntensity. E.g. sky({sun_elevation:0.4}) becomes
  sky({sunElevation:0.4}). Python and scene JSON stay snake_case.
- TS package: replace @usevoyager/vork in imports and package manifests with
  @voyager-so/vork. Install the SDK from the native archive's sdk directory;
  this guide does not imply a registry release. See vork help sdk.

SDK/binary versions and rollback
- The 0.2 TS/Python render helpers accept 0.1 metadata without warnings and return
  warnings: []. That does not mean 0.1 rendered every requested effect. Select
  the intended executable with VORK_BIN; new commands such as init need 0.2.
- writeScene/write_scene no longer insert $schema; explicit $schema is preserved.
  0.1 rejects this editor-only root field: remove it from scenes before rollback,
  including scenes made with vork init. Keep 0.1 source scenes and SDKs when
  rollback matters; 0.2-only features are not translated to 0.1 automatically.

Visible changes (recheck representative frames)
- Default std/final and std/draft denoising now filters in display-exposure units.
  Dim and high-exposure scenes can change substantially; recheck exposure and
  low-sample spectral detail. Undenoised reconstruction also changed, including
  denoise=false and the fallback on machines without GPU OIDN.
- Supplied mesh normals opposing triangle winding are aligned before shading;
  zero/degenerate normals use the face normal. Previously black metals can light
  correctly, and imported or hand-authored meshes can shade differently.
- std/path no longer counts finite-source lighting again at matched null-solid
  boundaries. One/two such slabs could previously produce 3x/5x overcounts.
  This correction changes path brightness; it does not change std/final.

Automation and performance
- Render stdout includes a Sidecars: line; setup starts with the vork version,
  OS: and FFmpeg version: lines. Progress goes to stderr every five seconds;
  the old "Shader cache is cold..." hint is gone. Do not parse by line position
  or treat any stderr as failure: check exit status, and use metadata sidecars
  or SDK results for exposure/warnings.
- Release-gate measurements found long interior/night std/final movies about
  4.3-4.4% slower, smaller cached edits slower, and first video about 2 s slower.
  Three cold corpus preparations exceeded the 150 s deadline. These are measured
  workloads, not universal costs; allow headroom in automation timeouts.
