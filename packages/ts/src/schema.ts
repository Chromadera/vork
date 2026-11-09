/** Generated from vork-scene Rust input types. Run npm run generate; do not edit. */

export type Layer = TrackLayer | ClipLayer;
export type Blend = "replace" | "add";
export type Interpolation = "STEP" | "LINEAR" | "CUBICSPLINE";
/**
 * Keyframes inline as `[time, value]` pairs, or two buffers for bulk data.
 */
export type Keys =
  | unknown[][]
  | {
      times: NameOr3;
      values: NameOr3;
    };
/**
 * A resource name or inline definition.
 */
export type NameOr3 = string | Buffer;
export type Buffer = InlineBuffer | FileBuffer | DenseBuffer;
/**
 * Inline numeric elements. Matrices contain 16 column-major components.
 */
export type InlineDataSchema = number[] | number[][];
export type BufferType = "f32" | "i32" | "u32" | "vec2" | "vec3" | "vec4" | "uvec4" | "mat4";
export type SubdivisionType = "catmull_clark" | "simple";
export type Topology = ("triangles" | "points" | "curves") | "implicit";
/**
 * Blender colour space names, verbatim.
 */
export type ColorSpace = "sRGB" | "Linear Rec.709" | "Non-Color";
export type MagFilter = "nearest" | "linear";
export type MinFilter =
  | "nearest"
  | "linear"
  | "nearest_mipmap_nearest"
  | "linear_mipmap_nearest"
  | "nearest_mipmap_linear"
  | "linear_mipmap_linear";
export type Background = [number, number, number] | BackgroundMode;
export type BackgroundMode = "environment" | "transparent";
/**
 * An ordered image-graph node. Resource names remain shorthand for an unnamed node.
 */
export type CompositeEntry = NameOr | CompositeNode;
/**
 * A resource name or inline definition.
 */
export type NameOr = string | Material;
/**
 * A material input value. Resources are explicit tagged objects; a plain string is
 * only ever an enum token of the program's reflected input type.
 */
export type Input =
  | number
  | number[]
  | boolean
  | TextureBinding
  | AttributeBinding
  | GridBinding
  | AovBinding
  | ChannelBinding
  | NodeBinding
  | string;
/**
 * A resource name or inline definition.
 */
export type NameOr4 = string | Image;
export type Channel = "r" | "g" | "b" | "a" | "rgb";
export type Factor = number | number[];
export type KernelChannel = "color" | "depth" | "view_normal" | "object_index" | "motion" | "shadow_catcher";
export type Opacity = "coverage" | "transmittance";
/**
 * A resource name or inline definition.
 */
export type NameOr2 = string | Program;
export type Program = FileProgram | SourceProgram;
/**
 * Number of threads, independent of the internal workgroup size.
 */
export type ComputeDispatch = number | [number, number, number];
export type ComputeSchedule = ("frame" | "install") | "simulation";
export type CompositeResolution = [number, number] | number;
export type Clock = number | string;
export type Wrap = "repeat" | "clamp_to_edge" | "mirrored_repeat";
/**
 * Translation, rotation and scale (glTF node TRS), flat on the node. Rotation is
 * stored as a quaternion; an authored `{"euler": [x,y,z]}` converts losslessly.
 */
export type Node = TransformNode | MeshNode | AssetNode | CameraNode | LightNode;
/**
 * An authored rotation: a glTF quaternion or a tagged Euler triple.
 */
export type Rotation =
  | [number, number, number, number]
  | {
      /**
       * Radians about X, then Y, then Z (fixed parent axes; Blender XYZ).
       *
       * @minItems 3
       * @maxItems 3
       */
      euler: [number, number, number];
    };
/**
 * Aiming references a node origin or a fixed point in world coordinates.
 */
export type AimTarget = string | [number, number, number];
/**
 * A resource name or inline definition.
 */
export type NameOr5 = string | Geometry;
export type LightShape = "sun" | "point" | "spot" | "rectangle" | "ellipse" | "environment";
export type AovType = "value" | "color";
/**
 * Output rate: decimal number or reduced rational string (frames per second).
 */
export type Time = number | string;
export type MotionBlurPosition = "start" | "center" | "end";

export interface Scene {
  /**
   * Editor JSON Schema address; metadata only, never fetched by the renderer.
   */
  $schema?: string | null;
  animation?: Layer[];
  buffers?: {
    [k: string]: Buffer;
  };
  geometry?: {
    [k: string]: Geometry;
  };
  images?: {
    [k: string]: Image;
  };
  materials?: {
    [k: string]: Material;
  };
  nodes: Node[];
  programs?: {
    [k: string]: Program;
  };
  /**
   * Authored straight segments inside a world-space box.
   */
  propagation?: Propagation | null;
  render?: Render;
  /**
   * Fixed-clock retained compute state, independent of output FPS.
   */
  simulation?: Simulation | null;
  version: 2;
}
export interface TrackLayer {
  blend?: Blend;
  clip?: null;
  loop?: boolean;
  name?: string | null;
  offset?: number;
  speed?: number;
  tracks: Track[];
  weight?: number;
}
/**
 * A keyed property.
 */
export interface Track {
  interpolation?: Interpolation | null;
  keys: Keys;
  /**
   * Segments naming the keyed property, e.g. ["nodes","Bone.001","rotation"].
   */
  path: string[];
}
export interface InlineBuffer {
  count?: number | null;
  data: InlineDataSchema;
  dims?: null;
  file?: null;
  /**
   * OpenVDB/NanoVDB field name; buffer.type describes voxel values.
   */
  grid?: string | null;
  offset?: number;
  type: BufferType;
}
export interface FileBuffer {
  count?: number | null;
  data?: null;
  dims?: null;
  file: string;
  /**
   * OpenVDB/NanoVDB field name; buffer.type describes voxel values.
   */
  grid?: string | null;
  offset?: number;
  type: BufferType;
}
export interface DenseBuffer {
  count?: null;
  data?: InlineDataSchema | null;
  /**
   * Dense typed array, x contiguous. Initially zero unless data is supplied.
   * Field consumers interpret cell centres in local [-1,1]³.
   *
   * @minItems 3
   * @maxItems 3
   */
  dims: [number, number, number];
  file?: null;
  grid?: null;
  offset?: number;
  type: "f32" | "vec3";
}
export interface ClipLayer {
  blend?: Blend;
  clip: Clip;
  loop?: boolean;
  name?: string | null;
  offset?: number;
  speed?: number;
  tracks?: null;
  weight?: number;
}
export interface Clip {
  file?: string | null;
  index?: number | null;
  name?: string | null;
  target: string;
}
export interface Geometry {
  attributes: {
    [k: string]: NameOr3;
  };
  /**
   * Live point prefix within the installed POSITION/width capacity.
   */
  count?: NameOr3 | null;
  /**
   * Endpoint vertex indices and OpenSubdiv/USD edge sharpness. Used by std subdivision.
   */
  crease_edge?: EdgeCrease[];
  /**
   * USD-style point counts, one u32 per polyline curve.
   */
  curve_vertex_counts?: NameOr3 | null;
  /**
   * Corners per polygon in the index buffer (USD/OpenSubdiv cage); absent means triangles.
   */
  face_vertex_counts?: number[] | null;
  /**
   * Temporal point identities; a changed identity is a birth, even when a row is reused.
   */
  ids?: NameOr3 | null;
  indices?: NameOr3 | null;
  material_ranges?: MaterialRange[];
  morphs?: {
    [k: string]: Morph;
  };
  /**
   * Blender Subdivision Surface, evaluated after deformation and before displacement.
   */
  subdivision?: SubdivisionSurface | null;
  topology?: Topology;
  /**
   * USD-style diameter in metres, one f32 per control point.
   */
  widths?: NameOr3 | null;
}
/**
 * An undirected cage edge. Crease is OpenSubdiv sharpness: 0 is smooth; 10 or greater is infinitely sharp.
 */
export interface EdgeCrease {
  crease: number;
  /**
   * @minItems 2
   * @maxItems 2
   */
  vertices: [number, number];
}
export interface MaterialRange {
  count: number;
  material_index: number;
  start: number;
}
export interface Morph {
  NORMAL?: NameOr3 | null;
  POSITION: NameOr3;
}
export interface SubdivisionSurface {
  render_levels?: number;
  subdivision_type?: SubdivisionType;
  use_limit_surface?: boolean;
}
export interface Image {
  /**
   * Blender `Image.colorspace_settings.name`. Omitted follows Blender's file default.
   */
  color_space?: ColorSpace | null;
  file?: string | null;
  mag_filter?: MagFilter;
  min_filter?: MinFilter;
  render?: ImageRender | null;
  /**
   * Seconds into media; omitted follows scene time. EXR sequences honor
   * framesPerSecond metadata; other sequences use render.fps.
   */
  time?: Clock | null;
  wrap_s?: Wrap;
  wrap_t?: Wrap;
}
/**
 * Settings that a nested image may author. The parent owns transport and time;
 * a capture has a closed shutter and runs the automatic per-step compositor.
 */
export interface ImageRender {
  background?: Background;
  camera?: string | null;
  compositor?: CompositeEntry[];
  /**
   * @minItems 2
   * @maxItems 2
   */
  resolution?: [number, number];
  root?: string | null;
}
/**
 * A program instance. Its surface entry shades the assigned geometry.
 */
export interface Material {
  double_sided?: boolean;
  inputs?: {
    [k: string]: Input;
  };
  /**
   * Meaning of geometry_opacity; the renderer owns its integration method.
   */
  opacity?: Opacity | null;
  program?: NameOr2;
}
/**
 * An image handle. Sampling and transformations belong to the receiving program.
 */
export interface TextureBinding {
  image: NameOr4;
}
/**
 * Install-resolved named float vertex data. Custom programs declare Attribute<float>
 * through Attribute<float4>; read_attribute returns value, dx and dy. Missing or
 * incompatible streams on any bound geometry fail install.
 */
export interface AttributeBinding {
  attribute: string;
  channel?: Channel | null;
  factor?: Factor | null;
}
/**
 * A volume grid buffer for a FloatGrid or VectorGrid program input.
 */
export interface GridBinding {
  grid: NameOr3;
}
/**
 * A render.aovs entry: an Aov handle for surface programs, or the AOV image in compositor programs.
 */
export interface AovBinding {
  aov: string;
}
/**
 * A kernel image channel read by a compositor program.
 */
export interface ChannelBinding {
  channel: KernelChannel;
}
/**
 * An earlier named compositor entry, local to this graph.
 */
export interface NodeBinding {
  node: string;
}
export interface FileProgram {
  bindings?: {
    [k: string]: NameOr3;
  };
  dispatch?: ComputeDispatch | null;
  file: string;
  schedule?: ComputeSchedule | null;
  source?: null;
  source_origin?: null;
}
export interface SourceProgram {
  bindings?: {
    [k: string]: NameOr3;
  };
  dispatch?: ComputeDispatch | null;
  file?: null;
  schedule?: ComputeSchedule | null;
  source: string;
  source_origin?: string | null;
}
export interface CompositeNode {
  material: NameOr;
  name?: string | null;
  resolution?: CompositeResolution | null;
}
export interface TransformNode {
  children?: Node[];
  /**
   * Cycles holdout: transparent black on transparent film; opaque black otherwise.
   */
  is_holdout?: boolean;
  /**
   * Cycles shadow catcher. Combined treats this surface as a holdout.
   */
  is_shadow_catcher?: boolean;
  light_link_tags?: string[];
  name?: string;
  pass_index?: number | null;
  rotation?: Rotation;
  /**
   * @minItems 3
   * @maxItems 3
   */
  scale?: [number, number, number];
  target?: AimTarget | null;
  /**
   * @minItems 3
   * @maxItems 3
   */
  translation?: [number, number, number];
  type?: "node";
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
export interface MeshNode {
  children?: Node[];
  geometry: NameOr5;
  instances?: Instances | null;
  /**
   * Cycles holdout: transparent black on transparent film; opaque black otherwise.
   */
  is_holdout?: boolean;
  /**
   * Cycles shadow catcher. Combined treats this surface as a holdout.
   */
  is_shadow_catcher?: boolean;
  light_link_tags?: string[];
  materials?: NameOr[];
  morph_weights?: {
    [k: string]: number;
  };
  name?: string;
  pass_index?: number | null;
  rotation?: Rotation;
  /**
   * @minItems 3
   * @maxItems 3
   */
  scale?: [number, number, number];
  skin?: Skin | null;
  target?: AimTarget | null;
  /**
   * @minItems 3
   * @maxItems 3
   */
  translation?: [number, number, number];
  type: "mesh";
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
export interface Instances {
  attributes: {
    [k: string]: NameOr3;
  };
  count?: NameOr3 | null;
}
export interface Skin {
  inverse_bind_matrices: NameOr3;
  joints: string[];
}
export interface AssetNode {
  children?: Node[];
  file: string;
  /**
   * Cycles holdout: transparent black on transparent film; opaque black otherwise.
   */
  is_holdout?: boolean;
  /**
   * Cycles shadow catcher. Combined treats this surface as a holdout.
   */
  is_shadow_catcher?: boolean;
  light_link_tags?: string[];
  materials?: {
    [k: string]: NameOr;
  };
  name?: string;
  nodes?: {
    [k: string]: AssetNodeOverrides;
  };
  pass_index?: number | null;
  rotation?: Rotation;
  /**
   * @minItems 3
   * @maxItems 3
   */
  scale?: [number, number, number];
  target?: AimTarget | null;
  /**
   * @minItems 3
   * @maxItems 3
   */
  translation?: [number, number, number];
  type: "asset";
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
/**
 * Visibility is ANDed with ancestors. Reflections and diffuse transport query
 * scene geometry independently of camera visibility. Diffuse visibility controls AO
 * and indirect occlusion; specular visibility controls reflected appearance.
 */
export interface AssetNodeOverrides {
  light_link_tags?: string[] | null;
  pass_index?: number | null;
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
export interface CameraNode {
  children?: Node[];
  focus_object?: string | null;
  /**
   * Cycles holdout: transparent black on transparent film; opaque black otherwise.
   */
  is_holdout?: boolean;
  /**
   * Cycles shadow catcher. Combined treats this surface as a holdout.
   */
  is_shadow_catcher?: boolean;
  light_link_tags?: string[];
  material?: NameOr | null;
  name?: string;
  pass_index?: number | null;
  rotation?: Rotation;
  /**
   * @minItems 3
   * @maxItems 3
   */
  scale?: [number, number, number];
  target?: AimTarget | null;
  /**
   * @minItems 3
   * @maxItems 3
   */
  translation?: [number, number, number];
  type: "camera";
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
export interface LightNode {
  angle?: number;
  children?: Node[];
  /**
   * None: LightShape::default_color (white, or Blender's world colour for an
   * environment without an image).
   *
   * @minItems 3
   * @maxItems 3
   */
  color?: [number, number, number] | null;
  energy?: number | null;
  illuminates?: string[] | null;
  image?: NameOr4 | null;
  /**
   * Cycles holdout: transparent black on transparent film; opaque black otherwise.
   */
  is_holdout?: boolean;
  /**
   * Cycles shadow catcher. Combined treats this surface as a holdout.
   */
  is_shadow_catcher?: boolean;
  light_link_tags?: string[];
  /**
   * Environment radiance material instance.
   */
  material?: NameOr | null;
  name?: string;
  pass_index?: number | null;
  radius?: number;
  rotation?: Rotation;
  /**
   * @minItems 3
   * @maxItems 3
   */
  scale?: [number, number, number];
  shadowed_by?: string[] | null;
  shape: LightShape;
  size?: number;
  size_y?: number | null;
  spot_blend?: number;
  spot_size?: number;
  target?: AimTarget | null;
  /**
   * None: `color` alone. Some(kelvin): `color` × std.color blackbody(kelvin).
   */
  temperature?: number | null;
  /**
   * @minItems 3
   * @maxItems 3
   */
  translation?: [number, number, number];
  type: "light";
  visible?: boolean;
  visible_camera?: boolean;
  visible_diffuse?: boolean;
  visible_glossy?: boolean;
  visible_shadow?: boolean;
  visible_transmission?: boolean;
}
export interface Propagation {
  /**
   * World-space minimum and maximum in metres; rays leaving continue straight.
   *
   * @minItems 2
   * @maxItems 2
   */
  bounds: [[number, number, number], [number, number, number]];
  /**
   * Program exporting PropagationStep propagation(PropagationContext, Inputs).
   */
  program: string;
}
export interface Render {
  /**
   * Shader AOV declarations; color outputs contain scene-linear RGB.
   */
  aovs?: Aov[];
  background?: Background;
  camera?: string | null;
  compositor?: CompositeEntry[];
  display?: NameOr;
  duration?: Clock;
  fps?: Time;
  motion_blur?: MotionBlur | null;
  /**
   * @minItems 2
   * @maxItems 2
   */
  resolution?: [number, number];
  root?: string | null;
}
export interface Aov {
  name: string;
  type: AovType;
}
export interface MotionBlur {
  position?: MotionBlurPosition;
  shutter?: number;
}
/**
 * A deterministic fixed-clock schedule over ordinary typed named buffers.
 */
export interface Simulation {
  /**
   * Positive fixed tick duration, in seconds; independent of render FPS.
   */
  dt: number | string;
  /**
   * Ordered compute program names establishing S0.
   */
  init: string[];
  /**
   * Simulation randomness is independent of image sampling randomness.
   */
  seed: number;
  /**
   * Complete persistent state and scratch buffers used by these passes.
   */
  state: string[];
  /**
   * Ordered compute program names advancing Sn to Sn+1.
   */
  steps: string[];
  /**
   * Time of the initialized state S0, in seconds.
   */
  t0: number | string;
}

/**
 * Profile file contents.
 */
export interface ProfileFile {
  description?: string;
  /**
   * Quality/cost level, independent of the profile's name: 0 draft, 1 final, 2 reference, 3 previs (a separate blocking look).
   */
  level: number;
  settings?: Settings;
  /**
   * The profile's own revision, recorded in output metadata.
   */
  version: number;
}
/**
 * Sampling seed, sample budget and denoising intent.
 */
export interface Settings {
  /**
   * Enable optional scene-linear OIDN. Off retains profile-owned reconstruction.
   */
  denoise?: boolean;
  /**
   * Override the profile sample budget; more samples means cleaner and slower.
   */
  samples?: number;
  /**
   * Deterministic sampling seed, independent of the selected technique.
   */
  seed?: number;
}
