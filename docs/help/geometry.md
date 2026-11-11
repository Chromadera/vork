GEOMETRY
SHAPES AND REVOLVE
Use std shape shorthand directly in a mesh node's geometry, for example:
{"type":"mesh","geometry":{"std":"revolve","profile":[[0,0],[0.4,0],[0.4,1],[0,1]],"segments":64}}
profile is an ordered list of [radius,height] points in metres, revolved around +Y.
Put radius-zero endpoints on the axis to close the ends. The profile traces the
surface; it is not a list of XYZ vertices. Other builders include box, sphere,
grid, cylinder, cone, torus, capsule, rounded_box, extrude, sweep and text.
`vork help std --all` lists builder signatures; `vork std revolve '{"profile":[[0,0],[1,0],[1,1],[0,1]]}'`
returns a geometry fragment. In Python use std("revolve", profile=[...]);
in TypeScript use revolve("vase", {profile:[...]}). See `vork help sdk` for SDK setup.

Instance columns are node-relative.

COLOR_n buffers accept linear RGB (vec3, alpha 1) or RGBA (vec4).
Use floating point colour components in 0..1, not byte values in 0..255;
glTF normalized unsigned byte/short accessors are decoded on import.
Bind authored colours explicitly, e.g. base_color: {"attribute":"COLOR_0"};
only imported glTF materials multiply base colour by COLOR_0 automatically.
Attribute values use the same linear Rec.709 space as constant material colours;
an sRGB picker value must be converted to linear before storing it.

Std extrude supports inward rounded rims: bevel_depth (metres), bevel_resolution
(quarter-circle steps). Total depth/bounds stay fixed.
Missing NORMAL attributes use face normals; authored NORMAL values control smoothing.
Std builders emit UVs but no TANGENT attribute.
Anisotropy uses TANGENT or material geometry_tangent; without either it uses a
fallback frame, not the UV direction.
Text size is the font em in metres, not visible cap height. Finite pixel/shutter
sampling can leave small strokes and bevel highlights unresolved; extra bevel
segments and denoising do not guarantee recovery. Samples cannot add missing geometry.

Normal maps use glTF/MikkTSpace: B = cross(N,T.xyz)*T.w. Derivative frames
recover handedness from UVs and the original normal, with no image-V sign flip.
Positive UV repeats/offsets preserve authored tangents; rotated, nonpositive-scale
or alternate-UV maps use derivatives. Missing tangents also use derivatives.
Use Non-Color normal textures; do not flip green to compensate for image V.

<!-- all -->
IMPLICIT SURFACE REFERENCE
Implicit geometry uses `topology: "implicit"` and exactly one attribute naming
an f32 dense buffer (`dims: [x,y,z]`, each >= 2). The attribute name is arbitrary.
The surface is the zero of the trilinearly interpolated field; negative is inside,
and the increasing-field gradient is the outward normal. The field need not be a
distance field. Samples are cell centres in local [-1,1]³, x contiguous;
node transforms place the domain. Boundary samples must be strictly positive.
Ordinary mesh materials shade the surface, including OpenPBR transmission.
Use surface materials without subsurface shading or displacement on implicit geometry.

Std builders expand in the author loader before scene decoding. Expanded geometry uses buffers; CPU expansion usage and coverage: `vork help std --all`.

GEOMETRY STAGE ORDER
scene.geometry_stages defaults to ["deform","compute","displace"]. Each named
stage may occur once. deform includes native morphs/skin and the existing fused
subdivision path; compute runs ordinary geometry programs, including cloth;
displace runs material displacement. For displacement before a solver, use
["deform","displace","compute"]. Canonical solver inputs sample their ordered
stateless predecessor prefix at each fixed-clock tick, including replay.
This does not admit compute-produced subdivision cages or computed colliders;
use native fixed triangles or a native collider proxy as diagnostics request.
Every stage required by an authored mesh must be listed; otherwise the error
names the node and the stage to add. Unused stages may be omitted, including an
empty list for scenes without geometry effects.

Sweep: {std:"sweep",profile:[[x,y],...],path:[[x,y,z],...]}; metres.
Repeat the first profile point for a closed section with caps; otherwise a strip.
Repeat the first path point for a closed loop without caps. Optional scale is a
positive number or one value per path point; twist is total radians along the path.
Closed paths need matching endpoint scales and whole turns of twist.
See examples/sweep.json and `vork help std --all`.

Std extrude accepts SVG `d` instead of `outline`/`holes` (XY metres; no Y flip).
First subpath is the outline, remaining subpaths are holes; existing crossing/nesting
checks apply. Curves flatten at 1e-4 of the path extent.
