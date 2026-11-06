vork — headless 3D rendering from scene JSON. Start small, inspect the image, then refine.
Supported GPUs: Apple Silicon (Metal) and recent NVIDIA (Vulkan).
Unknown fields are errors; omit anything at its default.

FIRST IMAGE
Save this as shot.json:
```json
{"version":2,"nodes":[
  {"type":"mesh","geometry":{"std":"sphere","radius":1},
   "materials":[{"inputs":{"base_color":[0.12,0.35,0.65],"specular_roughness":0.3}}]},
  {"type":"mesh","translation":[0,-1,0],"geometry":{"std":"grid","size":[10,10]},
   "materials":[{"inputs":{"base_color":[0.35,0.35,0.35]}}]},
  {"type":"light","shape":"rectangle","translation":[-3,5,4],"target":[0,0,0],
   "energy":700,"size":4,"color":[1,0.92,0.8]},
  {"type":"camera","translation":[3,2,5],"target":[0,0,0]}
],"render":{"resolution":[640,480]}}
```
vork render shot.json --out shot.png --profile std/previs
Inspect framing, scale and shadows. Adjust camera target, light position and material colour.
vork render shot.json --out shot.png --profile std/final
Use std/path for accurate indirect lighting, glass and caustics. For glass panes set
geometry_thin_walled: true. RGB background changes the backdrop, not the lighting.
To import a textured model, replace the first mesh with
{"type":"asset","name":"model","file":"model.glb"}. Keep .gltf files with their referenced buffers and textures.
Adjust the camera target and distance to the asset's size. Paths are relative to shot.json.

Supported features and known limits: vork help limits. Source builds: vork help build.

RENDERING
vork render SCENE --out PATH [--at SECONDS | --frame-start N --frame-end N] [--profile P]
  [--set KEY=VALUE ...] [--channels LIST] [--exr-codec CODEC] [--diagnostics]
  repeat --out PATH; the extension picks the output: .png one still (t=0 unless --at); a directory
  gets every frame as NNNNNN.png; PATH/%06d.exr frames; .mp4; .mov (keeps alpha).
  --channels color (default), depth, object_index, view_normal, motion, camera, aov:NAME;
  all includes render.aovs; applies to every --out.
  --profile: std/previs (blocking), std/draft (quick scene-ray lighting), std/final (delivery, default), std/path (accuracy).
  --set samples=N overrides the profile budget; more samples means cleaner and slower.
<!-- generated-call-keys:start -->
--set KEY=VALUE accepts resolution, fps, seed, samples, denoise.
<!-- generated-call-keys:end -->
Values are JSON; resolution also accepts WxH or W,H.
vork std FUNCTION JSON                 JSON argument; '-' reads stdin; JSON result
vork setup                             GPU report and default-profile shader warmup
vork setup --all                       Warm all bundled programs and profiles
vork help [TOPIC [--all] | field PATH]  topic details or exact field/object lookup

CONVENTIONS
Units: metres, seconds, radians; lens/sensor mm; linear Rec.709.
<!-- generated-standards:start -->
animation: follows glTF 2.0 interpolation
assets: follows glTF 2.0
cameras: follows Blender physical camera (Blender 5.2.1)
colour: follows AgX as Blender 5.2.1
geometry: follows glTF 2.0 topology and attribute semantics
lights: follows Blender 5.2.1 normalized lights
materials: follows OpenPBR Surface 1.1.1
textures: follows glTF 2.0 samplers; KHR_texture_transform lowered on import
<!-- generated-standards:end -->
Coordinates: right-handed Y-up; camera local -Z. Names exclude '/'; paths are scene-relative.
Resources accept a name or inline definition. Named tables use {name: definition}.
Node type fields are flat siblings; type: node has only common fields.
Normal maps: tangent-space +Y; TEXCOORD v=0 is the image top. Topology and program code are fixed during rendering.

COMMON FIELDS (advanced fields: vork help TOPIC --all; exact field: vork help field PATH)
<!-- generated-api:start -->

Scene {
  animation: layer[] = []
  buffers: {name:buffer} = {}
  geometry: {name:geometry} = {}
  images: {name:image} = {}
  materials: {name:material} = {}
  nodes: node[]
  programs: {name:program} = {}
  render: render = {}
  version: 2
}

Node {
  children: node[] = []
  name: string = ""; globally unique, including imported nodes
  rotation: [x,y,z,w] quaternion | {euler:[x,y,z]} (Blender XYZ) = [0,0,0,1]
  scale: [x,y,z] = [1,1,1]; any zero world-scale component hides geometry, including children
  target: node-name|[x,y,z]|null; aims -Z, world Y up; rotation is an offset on top
  translation: [x,y,z] = [0,0,0]; parent frame
  visible: bool = true; false hides the subtree and disables its lights; animatable (step)
  visible_camera: bool = true
  type: node|mesh|asset|camera|light = "node"
}

Mesh (type: mesh) {
  geometry: geometry-name|inline geometry
  materials: (material-name|inline material)[] = []; empty = default std/openpbr with roughness 0.5
}

Asset (type: asset) {
  file: file-path
}

Camera (type: camera) {
  focus_object: node-name|null = null; camera-axis depth supplied to the program at sample time
  material: material-name|inline material|null = null; omitted = std/camera; optics are program inputs
}

Light (type: light) {
  color: RGB
  energy: number|null; sun W/m²; local W; environment multiplier
  material: material-name|inline material|null; environment program
  shape: sun|point|spot|rectangle|ellipse|environment
  size: number = 1; rectangle full side, ellipse diameter
}

Material {
  double_sided: bool = true
  inputs: {name: number|vector|bool|enum-token|{image}|{attribute}|{grid}|{aov}|{node}|{channel}} = {}
  opacity: coverage|transmittance|null = null; null = coverage, transmittance for transmitting surfaces; coverage = occupied area fraction; transmittance = continuous attenuation by 1 - geometry_opacity; renderer chooses integration
  program: program-name|inline program|<library>/<module> = std/openpbr
}
std/openpbr inputs: base_weight base_color base_metalness specular_weight specular_color specular_roughness specular_roughness_anisotropy specular_ior transmission_weight transmission_color transmission_depth subsurface_weight subsurface_color subsurface_radius subsurface_radius_scale subsurface_scatter_anisotropy coat_weight coat_color coat_roughness coat_ior coat_darkening fuzz_weight fuzz_color fuzz_roughness emission_luminance emission_color thin_film_weight thin_film_thickness thin_film_ior geometry_opacity geometry_thin_walled geometry_normal geometry_tangent geometry_coat_normal transmission_dispersion_scale transmission_dispersion_abbe_number alpha_cutoff

Texture binding {
  image: image-name|inline image; the program owns sampling and UV transforms
}

Image {
  color_space: sRGB|Linear Rec.709|Non-Color|null = null; null: hdr/exr/ies linear, others sRGB
  file: file-path; XOR render; png jpg tif hdr exr ies mov mp4 mkv or %04d frame pattern; 8-bit may be block-compressed, use 16-bit for exact data; ies: LM-63 Type C
}

Render {
  background: environment|transparent|RGB = environment; RGB and transparent apply to camera escapes and opacity passthrough; path glass transmission sees the World
  camera: camera-node-name|null = null; null = the only authored camera, else first imported; a camera cut track may select
  compositor: (material-name|{material,name?,resolution?})[] = []; ordered image graph, last output; material may be inline
  display: material-name|inline material = {"program":"std/agx"}; display program; EXR stays scene-linear
  duration: number = 10.416667
  fps: number = 24
  motion_blur: MotionBlur|null = null; non-null enables; shutter in frames
  motion_blur.shutter: number = 0.5; frames
  resolution: uvec2 = [1920,1080]
}
<!-- generated-api:end -->
Asset nodes also take materials and nodes overrides (vork help nodes).

LIBRARIES
Std builders return {buffers,geometry}; author loaders expand {std: NAME, ...options} before scene decoding.
Shapes are centred; dimensions are metres. Inline geometry uses {std: NAME, ...options}.
Common builders (all shapes, text, extrusion and scatter: vork help std --all):
<!-- generated-shapes:start -->
box: {name?:string="box",size?:vec3=[2,2,2]} → GeometryFragment
sphere: {name?:string="sphere",radius?:number=1,segments?:number=32,rings?:number=16} → GeometryFragment
grid: {name?:string="grid",size?:vec2=[2,2],segments?:vec2=[1,1]} → GeometryFragment; XZ plane facing +Y; size,segments:[x,z]; u along +X, v along +Z
<!-- generated-shapes:end -->
Compositor: ordered {material,name?,resolution?} image graph; material may be inline.
Image inputs bind earlier results with {node:name}; last entry is output. See vork help compositor.
Environment: light.material evaluates radiance; std/environment_image reads ordinary Image inputs.
vork std environment preprocesses HDR lighting; vork std sky declares atmosphere and an ordinary sun.

Topics (vork help TOPIC [--all]):
contract: when scenes are validated; what is fixed after install
nodes: glTF asset overrides and imported-node addresses, skins, morph targets, object ID assignment
cameras: sampled depth of field; OpenCV export
lights: environment lighting and sky
materials: glass, textures and subsurface scattering
geometry: polygons, subdivision creases, morphs
buffers: binary data and computed placement
images: render-to-texture (image.render), texture filtering
animation: glTF clips and retargeting, layer blending and loops, every animatable property
render: frame timing and render-call fields
profiles: quality levels, deterministic seed and denoising; profile files
compositor: image graph, resolutions, sampling and custom Slang entries
light-paths: camera rays, per-contribution sensors, optical length and automatic spectral transport
programs: writing custom Slang shaders (surface, emission, displacement, environment, camera, sensor, display, compute)
outputs: file layouts, sidecar metadata, channel encodings, EXR codecs
camera-json: OpenCV camera sidecar format
sdk: TypeScript and Python helpers
std: full std/sky and std/hair inputs; Slang modules (noise, ramps, bump)
blender: Filmic display, Blender node functions for Slang
fluid: APIC/FLIP liquid, Yu–Turk surfaces, wetting and whitewater
post: glare, vignette, lens distortion and white balance inputs
errors: exit codes
diagnostics: timing files, benchmark flags, environment variables
limits: supported hardware, lighting approximations and scene-size limits
build: source prerequisites, pinned toolchains and tests on macOS, Linux and Windows
