UPGRADE
Breaking changes, newest release first. Rewrite scenes/scripts; the temporary shim below expires in 0.6.

0.5.0 (from 0.4.0)
- Depth now stays on the nearest visible triangle at folded-mesh edges instead
  of averaging disconnected surfaces that share an object ID (BB109).
- Known limitation (BB108): CPU depth and object-ID outputs can report isolated
  hits outside nearly edge-on, subpixel triangles, with depth outside the
  triangle’s range. Do not use affected CPU control pixels as surface distances;
  verify these outputs on a hardware GPU or against the source geometry.
- Fluid scenes authored on 0.4.0 may omit the fluid_status compute binding.
  Throughout 0.4.x, absent receipts retain the 0.4.0 correction and failure
  publication path without diagnostic writes. Existing explicit
  status bindings retain their error reporting. To adopt status reporting, use
  the current water example’s status bindings, geometry.status references and
  seven-element correction buffer; see vork help fluid.
- Box, sphere, grid, cylinder, cone, torus, capsule and revolve are first-class
  std builders again, with the same geometry as 0.4.0. No migration is needed
  for these eight names; see vork help std --all. They have no removal deadline.
- Find std/bloom, post/white_balance, std.procedurals and std.vector references
  (slash program names or dot Slang imports). These remain available in
  0.5.x with structured compatibility warnings and are removed in 0.6. Rewrite
  bloom as an authored compositor pyramid, white balance as a chromatic-adaptation
  matrix, and procedural noise/bump as authored Slang; Blender-specific imports
  can use blender.procedurals and blender.vector. See vork help compositor.
- Find std.particles imports. They keep loading with a compatibility warning
  throughout 0.5.x; before 0.6 rewrite the import as fluid.particles. The particle
  surface mathematics is unchanged.
- Find color_ramp and RGB/HSV/HSL conversion calls imported from std.color.
  These helpers moved to blender.color in 0.5.0 and have no compatibility shim.
  Rewrite the imports to blender.color or author the colour maths. std.color
  retains blackbody and wavelength.
- std/path profile version 2 defaults to 64 samples instead of 4096, with or
  without denoising. Custom level-2 profiles without settings.samples also use
  64. --set samples=4096 restores the old budget. Explicit budgets are unchanged.

0.4 (from 0.3)
- std/medium_constant and std/medium_height are removed. Author a medium program
  returning physical coefficients; see vork help fog for its contract and limits.
  World fog binds to light.medium on the single environment; homogeneous finite fog
  binds to closed convex carriers. Fog selects complete-camera path transport.

0.3 (from 0.2)
Scene admission and scripts
- 0.3 does not support participating media (fog, haze, smoke or volumes).
  No compatibility aliases are provided. Removed std/fog and Blender volume programs still fail
  lookup, even in unused definitions. For fog-dependent shots, bake an image
  plate and composite it, or render the medium externally. For a deliberately
  dry alternative, remove medium-only nodes, bindings and definitions while
  preserving shared surface geometry/materials. See vork help limits.
- glTF node cycles now fail at import, including nodes outside the selected scene.
  Remove cyclic child references or re-export an acyclic hierarchy; selecting
  another scene in the same file does not bypass validation.
- Environment preprocessing now refuses generated EXR frames that alias source
  frames, including symlink aliases. Choose a separate output directory/stem.
  `vork std environment` fps accepts a positive number or rational string such as
  "30000/1001" consistently for stills and footage; fix invalid rates before retrying.
- Used medium bindings (including imported overrides), volume topology and extra
  environment lights now fail instead of rendering with those effects dropped.
  Remove medium-only nodes/bindings to keep the old no-medium result, or composite
  an externally rendered plate. Combine environments into one program/map.
- Previously ignored geometry effects now fail: computed-instance displacement,
  displacement on computed points/implicit geometry, normal-dependent point displacement,
  conflicting displacement materials/shared-vertex attributes, computed geometry
  with coverage/transparency, and computed transparent placements.
  Bake displaced geometry or placements, compute positions in the producer,
  split discontinuous vertices, or use opaque geometry. See vork help limits.
- Named geometry_opacity/alpha_cutoff attributes now affect camera and ray hits;
  computed instance material attributes now follow GPU updates, shutter samples
  and seeks. Custom surfaces with named opacity bindings retain their program and
  material animation instead of being replaced by default OpenPBR. Remove these
  bindings if you intended the old constant/default look.
- SDK render at and environmentImage/environment_image fps now take numbers,
  not rational strings. Rewrite at:"1/24" as at:1/24 (Python at=1/24), and
  fps:"30000/1001" as fps:30000/1001. Scene clock fields and CLI --at still accept
  rational strings; render --set fps also retains rational rates.
- SDK writers no longer insert $schema; explicit $schema is preserved. Add it
  explicitly if editor schema discovery depends on it. SDK metadata requires a
  warnings field; older binaries without it fail. Upgrade the binary and SDK
  together.
- SDK writeScene/write_scene retires obsolete generated buffer siblings referenced
  by the destination's previous JSON. Copy JSON AND its buffers for snapshots;
  serialize saves/reads at one destination. Unrelated files and symlinks are kept.
  New JSON/buffers use 0666 subject to umask; replacement JSON keeps its mode.
  Set a restrictive umask when creating private scene files.
- Numeric scene animation uses numeric seconds; exact clock arithmetic is retained
  for output frames, media selection, simulation ticks and discrete decisions.
  Recheck cut/STEP and footage boundaries under reversed/looped layers. Use rational
  strings for exact clock/rate requests (e.g. fps "30000/1001"); keep transforms,
  weights, speeds and shutter width numeric. See vork help animation.

Look changes (rerender representative frames)
- Overlapping solid dielectrics gain material.priority (integer, default 0), on
  the material beside inputs. Higher priority owns IOR and absorption; equal
  priorities keep last-entered ownership (simultaneous entries use solid order).
  Set priorities explicitly for overlapping glass/liquid. Touching faces now cross
  directly between the two media, fixing spurious air interfaces and attenuation;
  remove artificial air gaps used as workarounds and recheck glass. Thin sheets
  have no interior. See vork help materials --all and field material.priority.
- Normal maps now follow glTF/MikkTSpace: B = cross(N,T.xyz)*T.w; derivative
  frames add no image-V sign flip. Positive UV repeats/offsets preserve authored
  tangents. Remove compensating green-channel flips or wrong tangent.w signs;
  check mapped base/coat normals and anisotropy. See vork help geometry.
- Path indirect clamping now measures mean absolute RGB instead of the largest
  channel (limit remains 10). Bright saturated indirect light can become brighter;
  recheck exposure and caustics. Direct light and raster GI are unchanged by this fix.
- Raster ambient/GI now uses the dielectric diffuse response consistently with
  direct/path shading. Mixed metals, rough/high-IOR materials, thin subsurface and
  diffusion can change brightness/colour. Recheck lighting; do not retain material
  compensation for the old response. Path shading is unchanged by this fix.
- Compositor file/video samples are premultiplied before filtering, fixing colour
  fringes at alpha edges. Recheck plates; custom composites must return
  premultiplied RGBA. Reflected image reads determine dependencies automatically;
  image.render captures now share each parent shutter sample's pose/time and retain
  their scene producers even when the final compositor replaces camera beauty.
- Linked shadows, imported/bound opacity and computed instance attributes now use
  the intended hit/material data across frames. Recheck linked/transmitting shadows
  and animated material attributes; remove workarounds for stale/default values.

Outputs, CLI and setup
- MP4 now checks final composited alpha on every frame. Composite an opaque
  background or choose .mov for transparency; transparent film may produce MP4
  when the compositor makes it opaque. Failed overwrites invalidate old completion
  metadata and remove owned old frames/channels (including native non-UTF-8
  filenames); copy outputs elsewhere before rerendering if you need them.
  Serialize writers per destination; concurrent writers now fail "output already
  in use". Require success and fresh meta.json before consuming a shot.
- meta.json and diagnostics now use version 3; frame times are numeric evaluation
  seconds, FPS remains number/rational string, camera.json stays version 2.
  Update readers that require metadata version 2 or rational frame timestamps.
- Footage decoders now close after failure so a corrected source can retry.
  Fix missing/broken footage before retrying; partial output is not completion.
- init now advertises runnable TS/Python starters; TS adds a module package.json.
  Regenerate in a new directory (existing output files are refused), install the
  matching bundled SDK, and follow its README. See vork help init and sdk.
- setup now warms only the vork init starter at the default std/final profile;
  the ten-minute shader deadline may end successfully with remaining work deferred.
  Required GPU/tool checks still fail on errors or their own stage timeouts.
  Use --skip-warmup for checks only, --all for the bundled program/profile catalog
  and built-in descriptors without a shader deadline; rerun to continue cached work.
  Other scene kinds compile once on first use. Progress is native stderr,
  including SDK-driven calls.
- Setup tool-validation failures now report setup exit code 4, rather than stdout
  IO code 5. Update automation to check the documented exit codes (vork help errors).
- Installers now reject unsupported macOS before download, bound stalled transfers,
  and preserve/restore existing installs on staging failures. Follow printed recovery
  commands. Linux NVIDIA discovery also finds mounted drivers; working Vulkan no
  longer depends on nvidia-smi succeeding. Use setup diagnostics for ICD failures.
- Linux source builds may set VORK_SHADER_TARGETS=spirv to skip Metal shader
  compilation. Keep the pinned Slang toolchain; see vork help build.

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
- Removed compositor programs/imports post/vignette, post/lens_distortion,
  post/glare and std/highlight require authored composite programs.
  The old lens effect was RGB chromatic dispersion, not geometric distortion.
  Additive glare needs an explicit merge with the original; std/bloom remains
  supported with its own stage contract (vork help compositor).
- Participating media were ignored with warnings in the public 0.2 release.
  In 0.3 they are refused and the fog/volume library programs are removed;
  follow the 0.3 media instructions above, including unused definitions.
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

SDK/binary versions
- Use matching SDK and binary versions. Render helpers require the current
  completion metadata contract, including warnings. VORK_BIN selects the binary.
- writeScene/write_scene preserve explicit $schema but do not insert one.

Visible changes (recheck representative frames)
- Default std/final and std/draft denoising filters in renderer-owned scene-linear units, independently of display exposure.
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
