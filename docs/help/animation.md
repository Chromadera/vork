ANIMATION
Put ordered layers in scene.animation. A layer has tracks or a glTF clip:
```json
"animation":[{"tracks":[{"path":["nodes","ball","translation"],
  "keys":[[0,[0,0,0]],[2,[2,0,0]]],"interpolation":"LINEAR"}]}]
```
Name the node `ball`. Paths are segments, not dotted strings; material inputs use
["materials", "material-name", "inputs", "input-name"]. Key times are seconds,
increasing. Interpolation: STEP, LINEAR (numeric default), CUBICSPLINE; strings and
bools default to STEP. `vork help animation --all` lists clip and layer fields.
Euler keys interpolate angles; quaternions take the short arc. Tracks hold endpoints.
Settings and topology are not animatable. Loops repeat the layer span; add composes
rotations. Bools and camera names switch at effective layer weight >=0.5; zero-weight
cuts are inactive. Numeric nonstructural scene fields, including material and program inputs, use these same tracks.
Aiming and object focus use evaluated poses at the requested time, including
installation and shutter samples. Unused base poses may coincide; coincident
evaluated aiming origins are errors naming the node and sample time.

Camera target is structural, including [x,y,z], and cannot be animated. Set
"target":"aim", add {"name":"aim","translation":[0,0,0]} to nodes, then track
["nodes","aim","translation"] with vector keys, e.g. [[0,[0,0,0]],[2,[1,0,0]]].
