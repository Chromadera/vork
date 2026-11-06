SDK
Scenes are plain Python dicts or TypeScript objects with the JSON fields.
Python: from vork import std, write_scene, render; install from PREFIX/lib/vork/python.
TypeScript: @usevoyager/vork; import builders, writeScene and render.
Python std("sphere", name="ball", radius=0.5); TS sphere("ball", {radius:0.5}).
Builders return buffers and geometry maps to merge into the scene.
write_scene(scene, path, base_dir) / writeScene(scene, path, baseDir) rebase paths
and externalize large buffers. Keep the JSON with its referenced files.
render(scene_path, out, at=0) / render(path, {out, at:0}) return exposure summaries.
VORK_BIN selects the executable. Types: vork.types, TS Scene; schema: sdk/scene.schema.json.
