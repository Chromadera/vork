NODES
Visibility per ray type is ANDed with ancestors. pass_index is inherited; otherwise IDs
are assigned stably in meta.json (0 = background).
Assets load .glb/.gltf. Imported names: asset/node-name or asset/#index.
Imported joints are not target, focus or attachment addresses.
glTF hierarchies must be acyclic, including nodes outside the selected scene.
A cycle fails import with its node index; repair child references or re-export.

<!-- all -->
Painted backdrops: disable visible_shadow; disable other ray visibility when the card should appear only to the camera.
FROM BLENDER
  Y-up: Blender (x,y,z) -> vork (x,z,-y). Euler is Blender XYZ order about vork axes; a Blender Z
  turn is vork Y. Lights and cameras aim local -Z, so identity is horizontal; aim with target.
  Quaternions are [x,y,z,w] (Blender: w,x,y,z). Key times are seconds; frames are 0-based
  (Blender frame f -> t=(f-1)/fps). Interpolation defaults to LINEAR; Blender-like ease is
  CUBICSPLINE with [t,v] keys.
  std/openpbr: glass tint is transmission_color and SSS colour is subsurface_color (base_color tints
  neither); specular_weight 1 = Blender Specular IOR Level 0.5; thin_film_thickness is micrometres.
  An RGB render.background is camera-only; light the scene with an environment light.
  Path glass transmission sees the environment, including straight-through thin glass;
  opacity passthrough keeps the camera background (RGB or transparent).
  The std/sky shader supplies atmospheric radiance. vork std sky returns that atmosphere plus
  an ordinary declared sun light, which lights the scene and casts shadows.
  view_normal is camera space (Blender Normal: world space); motion
  is current-minus-previous with +Y down (Blender Vector: previous-minus-current).
