# @usevoyager/vork

Rust-generated scene types, JSON Schema and serialization. Node 22.18+; no runtime npm dependencies.
Run `npm ci && npm run build`.

Author plain objects with `satisfies Scene` (a type import). `writeScene(scene, path,
baseDir)` resolves nested asset/image/buffer/program/clip paths against baseDir (default current directory)
and writes them relative to the scene file. Large inline buffers use a sibling binary sidecar.
Move the scene together with its referenced resources to preserve those relative paths.
For unchanged paths, serialize with `JSON.stringify`. Run the native `vork render`
command on the JSON; native validation checks numeric ranges and scene relationships.
The JSON Schema is exported as `@usevoyager/vork/scene.schema.json`.

Rust scene types are the source of truth. After changing them, run `npm run generate`.
`npm run check:generated` fails if either generated artifact is stale.

Authoring guidance: `vork help sdk`; looks: `vork help post`.
Runnable starter: [orbit.ts](examples/orbit.ts). Scene reference: `vork --help`.

Inline typed geometry and instance buffers are declared on `Scene.buffers`.
Field reference: `vork help geometry --all`.

The standard library keeps triangulated shape generation (`box`,
`sphere`, `cylinder`, `cone`, `torus`, `capsule`, `roundedBox`, `grid`, `revolve`, and the
native `extrude`), uniform
surface-area `scatter`, and the native font loader `text`. These return ordinary
geometry or instance data. See `vork help std`.

`render('shot.json', {out: 'still.png', at: 0, profile: 'std/previs'})` runs `vork render` with
render-time options (`at`, `profile`, `set`, `channels`); the scene file is unchanged.

Write maths, camera moves, tracks, materials, settings and table merges directly
in authoring code. Generated scene types remain the complete input contract.

Custom Slang vertex inputs use `Attribute<float>` through `Attribute<float4>`.
Bind them with the generated `AttributeBinding` type, for example
`{attribute: "COLOR_0", channel: "rgb", factor: [1, 0.5, 1]}`.
`read_attribute(inputs.tint, ctx)` returns value, `dx` and `dy` in raster and ray
shading; installation checks names and types on every bound mesh.

Scene writes publish a complete immutable sidecar before atomically replacing the JSON.
Readers therefore see a matching pair, including during concurrent saves. Older sidecars
are retained for readers holding previous JSON snapshots; interrupted saves can also leave
unreferenced sidecars. Fresh files are private; replacement JSON preserves existing permissions.

`render()` prints the CLI exposure lines and returns `{exposure: [{output, exposure}]}`.
Each summary contains `frames`, `coverage` (mean alpha), `clipped`, `near_black`, `mean`, and
`worst_clipped`/`worst_near_black` (`share`, absolute `frame`). EXR and control-only
outputs have no entry. Statistics weight display RGB by alpha; empty coverage gives
null metrics and worst-frame entries. Clip averages exclude empty frames. Values
retain full precision from the output metadata.

Surface programs author emitted light once with OpenPBR `emission_color` and
`emission_luminance`; hits and light samples share that evaluation. Use `lit`
for authored glow that should not become a sampled light.
