use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BufferType {
    F32,
    I32,
    U32,
    Vec2,
    Vec3,
    Vec4,
    Uvec4,
    Mat4,
}
/// Standard vertex attributes and their packed types. COLOR_n also accepts Vec3.
pub const STANDARD_ATTRIBUTES: [(&str, BufferType); 8] = [
    ("POSITION", BufferType::Vec3),
    ("NORMAL", BufferType::Vec3),
    ("TANGENT", BufferType::Vec4),
    ("TEXCOORD_0", BufferType::Vec2),
    ("TEXCOORD_1", BufferType::Vec2),
    ("COLOR_0", BufferType::Vec4),
    ("JOINTS_0", BufferType::Uvec4),
    ("WEIGHTS_0", BufferType::Vec4),
];
/// glTF colour sets are linear RGB or RGBA; RGB has implicit alpha one.
pub fn is_color_attribute(name: &str) -> bool {
    name.strip_prefix("COLOR_").is_some_and(|set| !set.is_empty() && set.bytes().all(|b| b.is_ascii_digit()))
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Buffer {
    #[serde(rename = "type")]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.type", "x-help" = "f32|i32|u32|vec2|vec3|vec4|uvec4|mat4"))]
    pub element_type: BufferType,
    #[schemars(with = "Option<InlineDataSchema>")]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.data", "x-help" = "number[]|number[][]; XOR file; one typed element per outer entry; vector/matrix entries contain 2/3/4/16 scalar components, not a flat scalar stream; mat4 column-major"))]
    pub data: Option<Vec<serde_json::Value>>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.file", "x-help" = "file-path; little-endian, or .vdb/.nvdb with grid; XOR data"))]
    pub file: Option<String>,
    /// OpenVDB/NanoVDB field name; buffer.type describes voxel values.
    #[schemars(extend("x-source" = "OpenVDB/NanoVDB; USD OpenVDBAsset fieldName", "x-help-path" = "buffer.grid", "x-help" = "name|null; .vdb/.nvdb only; f32 or vec3 voxels"))]
    pub grid: Option<String>,
    /// Dense typed array, x contiguous. Initially zero unless data or a binary file is supplied.
    /// Field consumers interpret cell centres in local [-1,1]³.
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.dims", "x-help" = "[u32,u32,u32]|null; positive dense typed array dimensions; data or binary file optional; file count defaults to dims product and must match it; x contiguous; field cell centres span local [-1,1]³"))]
    pub dims: Option<[u32; 3]>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.offset", "x-help" = "u64; bytes, multiple of 4"))]
    pub offset: u64,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "buffer.count", "x-help" = "u32|null; file only"))]
    pub count: Option<u32>,
}
impl Buffer {
    pub fn is_volume_grid(&self) -> bool {
        (self.grid.is_some() || self.dims.is_some())
            && matches!(self.element_type, BufferType::F32 | BufferType::Vec3)
    }

    /// Element count from inline data, a file count, or dense dimensions.
    pub fn len(&self) -> Option<usize> {
        self.data
            .as_ref()
            .map(Vec::len)
            .or(self.count.map(|c| c as usize))
            .or_else(|| self.dims.and_then(|d| d.into_iter().try_fold(1usize, |n, v| n.checked_mul(v as usize))))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    #[default]
    Triangles,
    Points,
    Curves,
    // USD Volume: named fields reference OpenVDB/NanoVDB grid buffers.
    #[schemars(skip)]
    Volume,
    /// Zero surface of one scalar dense grid; negative values are inside.
    Implicit,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    #[serde(default)]
    #[schemars(extend("x-source" = "glTF 2.0", "x-help-path" = "geometry.topology", "x-help" = "triangles|points|curves|implicit; points render as spheres"))]
    pub topology: Topology,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.attributes", "x-help" = "{name: buffer-name|inline buffer}; POSITION required (glTF names and types; COLOR_n: linear vec3/vec4, alpha defaults to 1); implicit: exactly one f32 dense grid, zero threshold; custom f32/vec2/vec3/vec4 bind to material inputs"))]
    #[schemars(with = "Table<NameOr<Buffer>>")]
    pub attributes: Table<String>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.indices", "x-help" = "u32-buffer-name|inline buffer|null"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub indices: Option<String>,
    /// Corners per polygon in the index buffer (USD/OpenSubdiv cage); absent means triangles.
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.face_vertex_counts", "x-help" = "u32[]|null; polygon corner counts; absent = triangles"))]
    pub face_vertex_counts: Option<Vec<u32>>,
    /// Endpoint vertex indices and OpenSubdiv/USD edge sharpness. Used by std subdivision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.crease_edge", "x-help" = "{vertices:[u32,u32],crease:number}[]; OpenSubdiv sharpness, 0 smooth, ≥10 sharp"))]
    pub crease_edge: Vec<EdgeCrease>,
    /// Blender Subdivision Surface, evaluated after deformation and before displacement.
    #[schemars(extend("x-source" = "Blender 5.2.1 (offline render subset)", "x-help-path" = "geometry.subdivision", "x-help" = "{subdivision_type?,render_levels?,use_limit_surface?}|null; Blender Subdivision Surface; one material range"))]
    pub subdivision: Option<SubdivisionSurface>,
    /// USD-style point counts, one u32 per polyline curve.
    #[schemars(extend("x-source" = "USD (polyline subset)", "x-help-path" = "geometry.curve_vertex_counts", "x-help" = "u32 buffer; points per curve, ≥2; curves only"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub curve_vertex_counts: Option<String>,
    /// USD-style diameter in metres, one f32 per control point.
    #[schemars(extend("x-source" = "USD Points / BasisCurves", "x-help-path" = "geometry.widths", "x-help" = "f32 buffer; diameter per point; curves and points; required"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub widths: Option<String>,
    /// Temporal point identities; a changed identity is a birth, even when a row is reused.
    #[schemars(extend("x-source" = "vork", "x-help-path" = "geometry.ids", "x-help" = "u32 buffer|null; points only; one unique identity per live point; matches previous poses across compact rows; births use current-pose motion placeholders"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub ids: Option<String>,
    /// Live point prefix within the installed POSITION/width capacity.
    #[schemars(extend("x-source" = "vork", "x-help-path" = "geometry.count", "x-help" = "one-element u32 buffer|null; points only; overflow rejects the exposure"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub count: Option<String>,
    /// Solver-owned sticky failure receipt, collected with evaluated geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-source" = "vork", "x-help-path" = "geometry.status", "x-help" = "{buffer,message}|null; one u32, zero succeeds, nonzero rejects output with message and value; producer keeps failure sticky until reset"))]
    pub status: Option<GeometryStatus>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.material_ranges", "x-help" = "{start,count,material_index}[]; partition indices into node.materials slots; empty = one material"))]
    pub material_ranges: Vec<MaterialRange>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.morphs", "x-help" = "{name:{POSITION,NORMAL?}}"))]
    pub morphs: Table<Morph>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeometryStatus {
    #[schemars(extend("x-source" = "vork", "x-help-path" = "geometry.status.buffer", "x-help" = "one-element u32 buffer-name|inline buffer; zero succeeds, nonzero rejects the exposure"))]
    #[schemars(with = "NameOr<Buffer>")]
    pub buffer: String,
    #[schemars(extend("x-source" = "vork", "x-help-path" = "geometry.status.message", "x-help" = "nonempty failure message including producer identity and remedy; reported u32 value appended"))]
    pub message: String,
}
/// An undirected cage edge. Crease is OpenSubdiv sharpness: 0 is smooth; 10 or greater is infinitely sharp.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeCrease {
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.crease_edge.vertices", "x-help" = "[u32,u32]"))]
    pub vertices: [u32; 2],
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.crease_edge.crease", "x-help" = "number; ≥0"))]
    pub crease: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialRange {
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.material_ranges.start", "x-help" = "u32; first index"))]
    pub start: u32,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.material_ranges.count", "x-help" = "u32; whole triangles"))]
    pub count: u32,
    #[schemars(extend("x-source" = "Blender MeshPolygon.material_index", "x-help-path" = "geometry.material_ranges.material_index", "x-help" = "u32; node.materials slot"))]
    pub material_index: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct Morph {
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.morphs.POSITION", "x-help" = "vec3-buffer-name|inline buffer"))]
    #[schemars(with = "NameOr<Buffer>")]
    pub POSITION: String,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "geometry.morphs.NORMAL", "x-help" = "buffer-name|inline buffer|null"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub NORMAL: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Instances {
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.instances.count", "x-help" = "one-element u32 compute buffer; zero to attribute capacity; overflow rejects the exposure"))]
    #[schemars(with = "Option<NameOr<Buffer>>")]
    pub count: Option<String>,
    #[schemars(extend("x-source" = "glTF EXT_mesh_gpu_instancing; vork time_offset/pass_index", "x-help-path" = "node.mesh.instances.attributes", "x-help" = "equal-length buffers: TRANSLATION vec3, ROTATION vec4 (xyzw), SCALE vec3 (omitted = identity; any zero world-scale component hides the instance), time_offset f32, pass_index u32, custom columns bind to material inputs"))]
    #[schemars(with = "Table<NameOr<Buffer>>")]
    pub attributes: Table<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.skin.joints", "x-help" = "node-name[]"))]
    pub joints: Vec<String>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.skin.inverse_bind_matrices", "x-help" = "mat4-buffer-name|inline buffer"))]
    #[schemars(with = "NameOr<Buffer>")]
    pub inverse_bind_matrices: String,
}

/// Inline numeric elements. Matrices contain 16 column-major components.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
enum InlineDataSchema {
    Scalars(Vec<f64>),
    Vectors(Vec<Vec<f64>>),
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubdivisionSurface {
    #[serde(default)]
    #[schemars(extend("x-source" = "Blender 5.2.1 (offline render subset)", "x-help-path" = "geometry.subdivision.subdivision_type", "x-help" = "catmull_clark|simple"))]
    pub subdivision_type: SubdivisionType,
    #[serde(default = "constants::subdivision_render_levels")]
    #[schemars(extend("x-source" = "Blender 5.2.1 (offline render subset)", "x-help-path" = "geometry.subdivision.render_levels", "x-help" = "u32; refinement iterations; allocation checked against device limits"))]
    pub render_levels: u32,
    #[serde(default = "constants::subdivision_limit")]
    #[schemars(extend("x-source" = "Blender 5.2.1 (offline render subset)", "x-help-path" = "geometry.subdivision.use_limit_surface", "x-help" = "bool"))]
    pub use_limit_surface: bool,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubdivisionType { #[default] CatmullClark, Simple }
#[cfg(test)]
mod semantic_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn color_sets_accept_rgb_and_rgba() {
        for semantic in ["COLOR_0", "COLOR_1", "COLOR_7"] {
            for (ty, data) in [("vec3", json!([[0.1,0.2,0.3]])), ("vec4", json!([[0.1,0.2,0.3,0.4]]))] {
                let value = json!({"version":2,"nodes":[{"type":"camera"}],
                    "buffers":{"p":{"type":"vec3","data":[[0,0,0]]},"c":{"type":ty,"data":data}},
                    "geometry":{"mesh":{"attributes":{"POSITION":"p",semantic:"c"}}}});
                let scene: Scene = serde_json::from_value(value).unwrap();
                scene.validate().unwrap();
            }
        }
    }

    #[test]
    fn exact_gltf_semantics_validate_types_and_preserve_custom_names() {
        for (semantic, _) in STANDARD_ATTRIBUTES {
            let mut value = json!({"version":2,"nodes":[{"type":"camera"}],
                "buffers":{"p":{"type":"vec3","data":[[0,0,0]]},"bad":{"type":"f32","data":[1]}},
                "geometry":{"mesh":{"attributes":{"POSITION":"p","_temperature":"bad","freely_named":"bad"}}}});
            let scene: Scene = serde_json::from_value(value.clone()).unwrap();
            scene.validate().unwrap();
            value["geometry"]["mesh"]["attributes"][semantic] = json!("bad");
            let scene: Scene = serde_json::from_value(value).unwrap();
            let error = scene.validate().unwrap_err().to_string();
            assert!(error.contains(semantic) && error.contains("wrong type"), "{error}");
        }
    }
}

/// Authoring shorthand expanded by vork-std before kernel scene decoding.
/// Builder-specific options are validated by the selected standard-library builder.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct StdGeometry {
    pub std: String,
    /// Text font path, resolved relative to the scene.
    pub font: Option<String>,
    #[serde(flatten)]
    pub options: Table<serde_json::Value>,
}
