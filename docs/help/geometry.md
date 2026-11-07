GEOMETRY
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
For a rod from A to B, use std/cylinder with height = length(B-A), translate
its centre to (A+B)/2 and rotate its local +Y axis onto normalize(B-A).
Reject coincident endpoints. Choose a perpendicular axis with a cross product;
use a different reference axis when the first is parallel to the rod.
For a tube arc in XY, sample angle a from start to end and tube angle b in 0..2π:
position = [(radius+tube*cos(b))*cos(a), (radius+tube*cos(b))*sin(a), tube*sin(b)].
Normal = [cos(b)*cos(a), cos(b)*sin(a), sin(b)]; connect adjacent rings with triangles.
Duplicate UV seams, cap partial arcs, and reverse winding for a negative sweep.
Procedural materials: write a Slang surface program (vork help programs --all);
use world/object position and std/procedurals rather than loading a sample scene.

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

Std builders expand in the author loader before scene decoding. Expanded geometry uses buffers; `vork std expand` emits buffer geometry (font paths relative to the working directory).
