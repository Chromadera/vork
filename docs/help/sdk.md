SDK
Start with vork init my-shot --language typescript (or --language python).
scene.json renders immediately; scene.ts/scene.py regenerates it with the matching SDK.
Scenes are plain Python dicts or TypeScript objects with the JSON fields.
Python: from vork import std, write_scene, render; install from PREFIX/lib/vork/python.
TypeScript: @voyager-so/vork; import builders, writeScene and render.
Python std("sphere", name="ball", radius=0.5); TS sphere("ball", {radius:0.5}).
Builders return buffers and geometry maps to merge into the scene.
write_scene(scene, path, base_dir) / writeScene(scene, path, baseDir) rebase paths
and externalize large buffers. Keep the JSON with its referenced files.
render(scene_path, out, at=0) / render(path, {out, at:0}) return exposure summaries and warnings [{code,requested,dropped}] for omitted effects.
VORK_BIN selects the executable. Types: vork.types, TS Scene; schema: sdk/scene.schema.json in the archive (packages/ts/scene.schema.json in source).

The 0.2 SDK can render 0.1-compatible scenes with a 0.1 binary; absent warning
metadata returns an empty warnings list, not proof that all effects were rendered.
Writers preserve an explicit $schema but do not insert one. Omit $schema for 0.1.
New 0.2 commands/features still need a 0.2 binary. See vork help upgrade.
