ANIMATION
Ordered scene.animation layers contain tracks or a glTF clip. Track paths are arrays
of scene path segments. Key times are increasing seconds; tracks hold endpoints.
Interpolation: STEP, LINEAR (numeric default), CUBICSPLINE; strings and bools default to STEP.
Cubic keys contain [time,value] (zero tangents) or [time,value,in_tangent,out_tangent];
tangents are value units per second, scaled by segment duration during interpolation.
vork help animation --all lists clip and layer fields.
Euler keys interpolate angles; quaternions take the short arc. Tracks hold endpoints.
Settings and topology are not animatable. Loops repeat the layer span; add composes
rotations. Bools and camera names switch at effective layer weight >=0.5; zero-weight
cuts are inactive. A render.camera cut track can supply the initial camera, so
render.camera may be omitted. Numeric nonstructural scene fields, including material and program inputs, use these same tracks.
Aiming and object focus use evaluated poses at the requested time, including
installation and shutter samples. Unused base poses may coincide; coincident
evaluated aiming origins are errors naming the node and sample time.

Camera target is structural, including [x,y,z], and cannot be animated. A named
node target follows that node's evaluated translation track.

Output frames keep integer identities with their FPS; simulation ticks keep
integer identities with t0 and dt. Ordinary transforms, materials, speeds,
offsets and weights remain numeric. Clock fields (duration, image.time, simulation t0/dt and key times) accept numeric
seconds or rational strings, such as "1/24" for a cut or "1/120" for simulation dt.
FPS accepts a number or rational rate, e.g. "30000/1001".
Rational strings are not numeric weights or transform values.

Media time retains exact STEP, LINEAR and cubic results through layer controls,
loops and ordered blends until selecting a source frame. Inline clock numbers
retain their authored decimal meaning; buffer clocks mean their stored IEEE
values. Numeric weights use the ordinary evaluator. A cut clamped from the left
stays on that side through retiming and previous-pose subtraction. Ordinary
continuous animation evaluates the endpoint value.
