MATERIALS
Use std/openpbr (the default) for constant, image and named attribute inputs.
Mesh Node.instances takes custom attribute columns in instances.attributes;
a material reads a column with {"attribute": name}. Two instances with different colours:
{"type":"mesh","geometry":{"std":"sphere"},
 "instances":{"attributes":{"TRANSLATION":{"type":"vec3","data":[[0,0,0],[2,0,0]]},
                            "tint":{"type":"vec3","data":[[1,0,0],[0,0,1]]}}},
 "materials":[{"inputs":{"base_color":{"attribute":"tint"}}}]}
These are data materials: they share the shipped evaluator and add no surface
code to the scene kernel. Use a surface program for custom logic, such as
procedural patterns, transformed texture sampling, or layered closures. Merely
returning OpenPBR parameters does not require a program.
emission_luminance is cd/m²; emission_color is linear RGB.
base_color is linear RGB; base_metalness=1
makes metal; specular_roughness controls highlight width. Inputs and defaults:
vork help field material.inputs
Image inputs bind {"image": name}; std/openpbr uses UV0, red for scalar inputs
and RGB for colour. Custom factors, channels and UV transforms need a program.
For glass panes use transmission_weight: 1 and geometry_thin_walled: true.
Use transmission_color for glass tint and subsurface_color for scattering colour.
Use std/path when refraction or indirect lighting is important.
In std/previs, scalar std/openpbr geometry_opacity and alpha_cutoff use each
material's raster coverage, retaining direct lighting and visible emission. Fractional
coverage is sampled and can look grainy at the preview sample budget. Bindings
or animation of either input, and opacity combined with transmission, retain
whole-view path transport until qualified. Other effects can also require path
transport. The sample budget and denoise setting stay unchanged.
Raster previs uses analytic lights and environment approximations. Emissive
meshes stay visible, but light they cast on other objects needs std/draft,
std/final or std/path; the former alpha-triggered path view included that light.
<!-- all -->
MATERIAL REFERENCE
Solid glass requires closed mesh boundaries; model hollow vessels with a shell.
For overlapping glass and liquid, assign material.priority (see vork help field
material.priority). The owning medium supplies both IOR and absorption; hidden
lower-priority boundaries pass straight through. Equal priorities preserve nested
replacement: the last entered solid wins. Touching faces cross directly between
the two media, without an air interface. Thin sheets have no interior.
With transmission_depth=0, a closed solid shows the authored transmission_color;
each thin sheet applies the full tint. Use thin-walled panes for windows.
Thin-walled subsurface suits leaves, paper and curtains: geometry_thin_walled=true,
subsurface_weight>0. Set subsurface_color and subsurface_scatter_anisotropy;
positive anisotropy favours transmission. Radius controls apply to thick objects.
On thick meshes, transmission_weight mixes glass with the subsurface substrate.
Thick subsurface transport freezes homogeneous coefficients at entry and uses an
index-matched diffuse boundary; zero mean-free-path channels retain local response.
geometry_normal and geometry_coat_normal accept tangent-space XYZ vectors or
linear RGB textures. An omitted coat normal stays unperturbed by the base normal map.
Normal-map frames follow glTF/MikkTSpace (vork help geometry).
For transformed normal maps, custom programs use std.texture.
Raster ambient/GI reserves the same dielectric diffuse response as direct/path
shading, including mixed metals and thin subsurface. Raster transport remains an
approximation; use std/path to assess indirect lighting.
Use constant, texture or named attribute bindings for geometry_opacity and
alpha_cutoff; camera and ray hits share evaluation, preserving custom surface
programs and their material animation. Computed instance material
attributes follow GPU publications, including shutter samples and seeks.
Numeric inputs animate with material tracks. Weighted/additive program tracks
start from the authored input, or zero when omitted. Untracked inputs retain
program initializers. Custom shading: vork help programs --all.
