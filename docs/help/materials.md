MATERIALS
Start with std/openpbr (the default). base_color is linear RGB; base_metalness=1
makes metal; specular_roughness controls highlight width. Inputs and defaults:
vork help field material.inputs
Image inputs bind {"image": name}; programs own factors, channels and UV transforms.
For glass panes use transmission_weight: 1 and geometry_thin_walled: true.
Use transmission_color for glass tint and subsurface_color for scattering colour.
Use std/path when refraction or indirect lighting is important.
<!-- all -->
MATERIAL REFERENCE
Solid glass requires closed mesh boundaries; model hollow vessels with a shell.
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
