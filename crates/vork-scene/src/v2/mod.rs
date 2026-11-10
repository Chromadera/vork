//! Version 2 authoring contract: the one scene model the renderer installs.
pub use vork_time::Time;
pub mod animation;
mod constants;
pub mod diagnostics;
pub mod geometry;
mod inline;
mod resources;
mod image_plan;
pub mod light;
pub mod material;
pub mod node;
pub mod profile;
pub mod render;
mod schema;
pub mod targets;
mod validate;
pub use animation::*;
use anyhow::{Context, Result, ensure};
pub use constants::OPENPBR;
pub use geometry::*;
pub use light::*;
pub use material::*;
pub use node::*;
pub use profile::*;
pub use render::*;
pub use schema::{api_field, documents, lookup_only_field, schema};
pub use validate::validate_openpbr_input;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
pub use targets::{NodeProperty, Ray, RenderProperty, TrackTarget, TrackValue, display_path};

// Serde buffers untagged/flattened fields through deserialize_any. Restore
// JSON's exact-number representation before lowering a numeric field to f32/f64.
pub(crate) fn numeric<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> std::result::Result<T, D::Error> {
    serde_json::from_value(serde_json::Value::deserialize(d)?).map_err(serde::de::Error::custom)
}

pub(crate) fn lower_numeric_json(value: &mut serde_json::Value) -> std::result::Result<(), &'static str> {
    match value {
        serde_json::Value::Number(number) if !number.is_i64() && !number.is_u64() => {
            *number = serde_json::Number::from_f64(number.as_f64().ok_or("numeric input must be finite")?)
                .ok_or("numeric input must be finite")?;
        }
        serde_json::Value::Array(values) => for value in values { lower_numeric_json(value)?; },
        serde_json::Value::Object(values) => for value in values.values_mut() { lower_numeric_json(value)?; },
        _ => {}
    }
    Ok(())
}

pub type Table<T> = BTreeMap<String, T>;
/// A resource name or inline definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NameOr<T> {
    Name(String),
    Inline(T),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("x-help-extra" = "coordinates: glTF right-handed Y-up; cameras look along local -Z"))]
pub struct Scene {
    /// Authored locations for diagnostics; never scene content or resource identity.
    #[serde(skip)]
    pub origins: std::collections::BTreeMap<String, Vec<String>>,
    /// Editor JSON Schema address; metadata only, never fetched by the renderer.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-help-path" = "scene.$schema", "x-help" = "string; editor schema URL"))]
    pub schema: Option<String>,
    /// Internal delivery admission/approximation policy; never scene content.
    #[serde(skip)]
    pub delivery: bool,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.version", "x-help" = "2"))]
    pub version: u32,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.render", "x-help" = "render"))]
    pub render: Render,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.nodes", "x-help" = "node[]"))]
    pub nodes: Vec<Node>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.animation", "x-help" = "layer[]"))]
    pub animation: Vec<Layer>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.buffers", "x-help" = "{name:buffer}"))]
    pub buffers: Table<Buffer>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.geometry", "x-help" = "{name:geometry}"))]
    pub geometry: Table<Geometry>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.images", "x-help" = "{name:image}"))]
    pub images: Table<Image>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.programs", "x-help" = "{name:program}"))]
    pub programs: Table<Program>,
    /// Ordered geometry lowerings; compute contains ordinary geometry programs.
    #[serde(default = "constants::geometry_stages")]
    #[schemars(extend("x-source" = "vork geometry chain", "x-help-path" = "scene.geometry_stages", "x-help" = "(deform|compute|displace)[]; ordered, each at most once; default [deform,compute,displace]"))]
    pub geometry_stages: Vec<GeometryStage>,
    /// Fixed-clock retained compute state, independent of output FPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-source" = "vork simulation contract", "x-help-path" = "scene.simulation", "x-help" = "simulation|null; fixed-clock ordered retained compute"))]
    pub simulation: Option<Simulation>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "scene.materials", "x-help" = "{name:material}"))]
    pub materials: Table<Material>,
    /// Authored straight segments inside a world-space box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-source" = "vork authored propagation", "x-help-path" = "scene.propagation", "x-help" = "{program,bounds}|null; authored straight ray segments inside world-space bounds"))]
    pub propagation: Option<Propagation>,
}
/// Native deformation (including subdivision), geometry compute, and displacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GeometryStage { Deform, Compute, Displace }
impl GeometryStage {
    pub const DEFAULT: [Self;3] = [Self::Deform,Self::Compute,Self::Displace];
    pub fn name(self)-> &'static str {match self {Self::Deform=>"deform",Self::Compute=>"compute",Self::Displace=>"displace"}}
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Propagation {
    /// Program exporting PropagationStep propagation(PropagationContext, Inputs).
    pub program: String,
    /// World-space minimum and maximum in metres; rays leaving continue straight.
    pub bounds: [[f32; 3]; 2],
}
impl Scene {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read scene {}", path.display()))?;
        Self::from_json_with_source(&bytes, &bytes, Some(path))
            .with_context(|| format!("load scene {}", path.display()))
    }
    /// A scene document; a leading UTF-8 byte-order mark (PowerShell 5.1 writes one) is ignored.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        Self::from_json_with_source(bytes, bytes, None)
    }
    /// Decode expanded geometry while retaining the original document's locations.
    pub fn from_json_with_source(bytes: &[u8], authored: &[u8], file: Option<&Path>) -> Result<Self> {
        Self::decode_with_source(bytes,authored,file,true)
    }
    /// Offline asset consumer: validate content without requiring a render camera.
    pub fn from_json_for_export(bytes:&[u8],authored:&[u8],file:Option<&Path>)->Result<Self> {
        Self::decode_with_source(bytes,authored,file,false)
    }
    fn decode_with_source(bytes:&[u8],authored:&[u8],file:Option<&Path>,require_camera:bool)->Result<Self> {
        let bytes = diagnostics::without_bom(bytes);
        let mut value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| diagnostics::decode_error(bytes, error))?;
        ensure!(value.pointer("/render/settings").is_none(),
            "render.settings: render settings are not scene content; choose them with --profile (vork help profiles)");
        ensure!(value.pointer("/render/integrator").is_none() && value.pointer("/render/samples").is_none(),
            "render.integrator and render.samples are render settings, not scene content; choose --profile std/draft, std/final or std/path (vork help profiles)");
        let mut origins = inline::lower(&mut value);
        inline::locate(diagnostics::without_bom(authored), &mut origins);
        if let Some(file) = file {
            for path in origins.values_mut().flatten() { *path = format!("{}: {path}", file.display()); }
        }
        let mut scene: Self = serde_json::from_value(value)
            .map_err(|error| diagnostics::decode_error(bytes, error))?;
        ensure!(!scene.materials.keys().any(|name| name.starts_with("__vork_")),
            "material names starting with __vork_ are reserved for the renderer");
        scene.origins = origins;
        scene.validate_content(require_camera).map_err(|error| scene.authored_error(error))?;
        Ok(scene)
    }
    /// Add authored users to errors without changing normalized resource identity.
    pub fn authored_error(&self, error: anyhow::Error) -> anyhow::Error {
        let text = format!("{error:#}");
        let programs = self.materials.iter().filter(|(name, _)| text.contains(&format!("material {name}:")) || text.contains(&format!("material {name} ")))
            .map(|(_, material)| format!("program {}", material.program)).collect::<Vec<_>>();
        let error = if programs.is_empty() { error } else { error.context(programs.join("; ")) };
        diagnostics::authored_error(&self.origins, error)
    }

    pub fn validate(&self) -> Result<()> { self.validate_content(true) }
    fn validate_content(&self,require_camera:bool)->Result<()> {
        if let Some(p) = &self.propagation {
            ensure!(self.programs.contains_key(&p.program), "unknown propagation program {}", p.program);
            ensure!(p.bounds.iter().flatten().all(|v|v.is_finite()) && (0..3).all(|i|p.bounds[0][i]<p.bounds[1][i]), "propagation.bounds must have finite ordered endpoints");
        }

        ensure!(
            self.version == 2,
            "version {}: only scene version 2 is supported",
            self.version
        );
        for names in [
            self.buffers.keys().collect::<Vec<_>>(),
            self.geometry.keys().collect(),
            self.images.keys().collect(),
            self.programs.keys().collect(),
            self.materials.keys().collect(),
        ] {
            for n in names {
                validate_name(n)?;
            }
        }
        let mut names = BTreeMap::new();
        let mut groups = std::collections::HashSet::new();
        let mut cameras = 0;
        let mut has_assets = false;
        fn visit<'a>(
            nodes: &'a [Node],
            names: &mut BTreeMap<&'a str, &'a Node>,
            groups: &mut std::collections::HashSet<&'a str>,
            cameras: &mut usize,
            has_assets: &mut bool,
        ) -> Result<()> {
            for n in nodes {
                if !n.name.is_empty() {
                    validate_name(&n.name)?;
                    ensure!(
                        names.insert(&n.name, n).is_none(),
                        "duplicate node name {:?}; node names must be unique",
                        n.name
                    );
                }
                ensure!(
                    n.pass_index.is_none_or(|v| (1..=65535).contains(&v)),
                    "node {:?}: pass_index 0 is reserved for background; use 1..65535",
                    n.name
                );
                if let NodeKind::Asset {
                    nodes: overrides, ..
                } = &n.kind
                {
                    *has_assets = true;
                    for (name, node) in overrides {
                        groups.extend(node.light_link_tags.iter().flatten().map(String::as_str));
                        ensure!(
                            node.pass_index.is_none_or(|v| (1..=65535).contains(&v)),
                            "asset override {name:?}: pass_index 0 is reserved for background; use 1..65535"
                        );
                    }
                }
                groups.extend(n.light_link_tags.iter().map(String::as_str));
                if matches!(n.kind, NodeKind::Camera { .. }) {
                    *cameras += 1;
                }
                visit(&n.children, names, groups, cameras, has_assets)?;
            }
            Ok(())
        }
        visit(&self.nodes, &mut names, &mut groups, &mut cameras, &mut has_assets)?;
        ensure!(!require_camera || cameras > 0 || has_assets, "scene requires at least one camera");
        fn validate_links(nodes: &[Node], groups: &std::collections::HashSet<&str>) -> Result<()> {
            for node in nodes {
                if let NodeKind::Light {
                    illuminates,
                    shadowed_by,
                    ..
                } = &node.kind
                {
                    for name in illuminates.iter().chain(shadowed_by.iter()).flatten() {
                        ensure!(
                            groups.contains(name.as_str()),
                            "{}",
                            diagnostics::unknown("light_link_tags entry", name, groups.iter().copied())
                        );
                    }
                }
                validate_links(&node.children, groups)?;
            }
            Ok(())
        }
        validate_links(&self.nodes, &groups)?;
        for b in self.buffers.values() {
            ensure!(
                b.dims.is_some() || b.data.is_some() != b.file.is_some(),
                "buffer requires dims or exactly one of data or file"
            );
        }
        for p in self.programs.values() {
            ensure!(
                p.source.is_some() != p.file.is_some(),
                "program requires exactly one of source or file"
            );
        }
        for g in self.geometry.values() {
            ensure!(
                matches!(g.topology, Topology::Volume | Topology::Implicit) || g.attributes.contains_key("POSITION"),
                "geometry requires POSITION"
            );
            for b in g.attributes.values().chain(g.indices.iter()) {
                ensure!(
                    self.buffers.contains_key(b),
                    "{}",
                    diagnostics::unknown("buffer", b, self.buffers.keys().map(String::as_str))
                );
            }
        }
        for l in &self.animation {
            ensure!(
                l.tracks.is_some() != l.clip.is_some(),
                "layer requires exactly one of tracks or clip"
            );
        }
        self.validate_image_sources(&names)?;
        self.validate_resources(&names,require_camera)
    }
}

pub(super) fn validate_name(s: &str) -> Result<()> {
    ensure!(!s.is_empty(), "name must not be empty");
    // '/' joins an asset name and an imported node name in diagnostics and IDs.
    ensure!(!s.contains('/'), "name {s:?}: '/' is not allowed in names");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_schema_is_optional_metadata() {
        for url in [crate::SCHEMA_URL, "https://voyager.so/vork/schema/scene.schema.json", "custom editor metadata"] {
            let input = serde_json::json!({"version": 2, "nodes": [], "$schema": url});
            let scene: Scene = serde_json::from_value(input).unwrap();
            assert_eq!(scene.schema.as_deref(), Some(url));
            assert_eq!(serde_json::to_value(scene).unwrap()["$schema"], url);
        }
        assert!(serde_json::from_value::<Scene>(serde_json::json!({"version": 2, "nodes": [], "$schem": "typo"})).is_err());
    }

    #[test]
    fn propagation_is_only_program_and_ordered_bounds() {
        let scene=serde_json::json!({"version":2,"nodes":[{"type":"camera"}],
            "programs":{"step":{"source":"struct Inputs {};"}},
            "propagation":{"program":"step","bounds":[[-1,-1,-1],[1,1,1]]}});
        assert!(Scene::from_json(&serde_json::to_vec(&scene).unwrap()).is_ok());
        for (field,value) in [("capture_distance",serde_json::json!(1)),("physical_scale",serde_json::json!(1)),
            ("radiance_scale",serde_json::json!(1)),("program",serde_json::json!("missing")),
            ("bounds",serde_json::json!([[1,-1,-1],[1,1,1]]))] {
            let mut invalid=scene.clone();invalid["propagation"][field]=value;
            assert!(Scene::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err(),"{invalid}");
        }
    }

    #[test]
    fn retired_technique_intent_is_not_scene_content() {
        for extra in [serde_json::json!({"requires":["path"]}),serde_json::json!({"materials":{"glass":{"thickness_mode":"slab"}}}),serde_json::json!({"programs":{"p":{"source":"import vork;","passes":[]}}}),serde_json::json!({"render":{"compositor_per_step":[]}})] {
            let mut scene=serde_json::json!({"version":2,"nodes":[{"type":"camera"}]});
            scene.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert!(Scene::from_json(&serde_json::to_vec(&scene).unwrap()).is_err(),"{scene}");
        }
    }
    #[cfg(all(test, vork_private_tests))]
    include!("mod_unavailable_path_volume_budget_is_absent_and_rejected_private.rs");

    #[test]
    fn camera_cut_tracks_defer_start_selection_to_animation_evaluation() {
        let text = br#"{"version":2,"nodes":[{"type":"camera","name":"wide"},{"type":"camera","name":"close"}],
            "animation":[{"tracks":[{"path":["render","camera"],"keys":[[0,"close"],[2,"wide"]]}]}]}"#;
        assert_eq!(Scene::from_json(text).unwrap().render.camera, None);
        let without = br#"{"version":2,"nodes":[{"type":"camera","name":"wide"},{"type":"camera","name":"close"}]}"#;
        assert!(Scene::from_json(without).unwrap_err().to_string().contains("requires render.camera"));
    }
    #[test]
    fn inactive_cut_layers_leave_the_unambiguous_camera_default() {
        for weight in [0., 0.25] {
            let scene = Scene::from_json(&serde_json::to_vec(&serde_json::json!({"version":2,
                "nodes":[{"type":"camera","name":"main"},{"type":"asset","name":"rig","file":"imported-camera.gltf"}],
                "animation":[{"weight":weight,"tracks":[{"path":["render","camera"],"keys":[[0,"rig/shot"]]}]}]
            })).unwrap()).unwrap();
            assert_eq!(scene.render.camera, None, "weight={weight}");
        }
    }
    #[test]
    fn offset_cut_layers_still_name_a_camera() {
        for offset in [-2., 2.] {
            let scene = Scene::from_json(&serde_json::to_vec(&serde_json::json!({"version":2,
                "nodes":[{"type":"camera","name":"a"},{"type":"camera","name":"b"}],
                "animation":[{"offset":offset,"tracks":[{"path":["render","camera"],"keys":[[0,"b"]]}]}]
            })).unwrap()).unwrap();
            assert_eq!(scene.render.camera, None);
        }
    }
    #[test]
    fn scene_json_accepts_a_leading_byte_order_mark() {
        let json = br#"{"version":2,"nodes":[{"type":"camera","name":"camera"}]}"#;
        let scene = Scene::from_json(&[b"\xEF\xBB\xBF".as_slice(), json].concat()).unwrap();
        assert_eq!(scene.nodes[0].name, "camera");
    }
    fn parse(v: serde_json::Value) -> Result<Scene> {
        let scene: Scene = crate::v2::from_test_value(v)?;
        scene.validate()?;
        Ok(scene)
    }
    #[test]
    fn probe_world_bounds_are_rejected_in_profiles() {
        for bounds in [serde_json::json!([[-4,-2,-3],[6,5,8]]), serde_json::Value::Null] {
            let error = Profile::parse("test", &serde_json::to_vec(&serde_json::json!({
                "version":1,"level":1,"settings":{"probes":{"volume":{"bounds":bounds}}}
            })).unwrap()).unwrap_err().to_string();
            assert!(error.contains("probes") && error.contains("unknown field"), "{error}");
        }
    }
    #[test]
    fn names_report_the_specific_failed_rule() {
        let check = |names: &[&str]| {
            let nodes: Vec<_> = names
                .iter()
                .map(|name| serde_json::json!({"type":"camera","name":name}))
                .collect();
            parse(serde_json::json!({"version":2,"nodes":nodes}))
                .unwrap_err()
                .to_string()
        };
        assert!(check(&["car/wheel"]).contains("'/' is not allowed"));
        assert!(check(&["camera", "camera"]).contains("duplicate node name"));
        parse(serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"Cube.001","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],"materials":{"a.b":{}}}))
            .unwrap();
    }
    #[test]
    fn rejects_unknown_fields_duplicate_names_and_ambiguous_sources() {
        let valid = serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"cam","material":{"program":"std/camera","inputs":{"projection":"persp"}}}]});
        parse(valid.clone()).unwrap();
        for invalid in [
            serde_json::json!({"version":2,"world":{},"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}]}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","look_att":[0,0,0],"material":{"program":"std/camera","inputs":{"projection":"persp"}}}]}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"a","material":{"program":"std/camera","inputs":{"projection":"persp"}}},{"type":"node","name":"a"}]}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],"buffers":{"p":{"type":"vec3","data":[],"file":"p.bin"}}}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],"programs":{"p":{}}}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],"animation":[{"tracks":[{"path":["render","camera"],"keys":[[1,"a"],[0,"b"]]}]}]}),
        ] {
            assert!(parse(invalid.clone()).is_err(), "accepted {invalid}");
        }
    }
    #[test]
    fn rejects_unimplemented_pass_fields() {
        let error = parse(serde_json::json!({"version":2,"nodes":[{"type":"camera"}],
            "render":{"pass_cryptomatte_depth":6}})).unwrap_err().to_string();
        assert!(error.contains("pass_cryptomatte_depth") && error.contains("unknown"), "{error}");
    }
    #[test]
    fn aov_names_never_shadow_built_in_channels() {
        let scene = |name: &str| parse(serde_json::json!({"version":2,"nodes":[{"type":"camera"}],
            "render":{"aovs":[{"name":name,"type":"value"}]}}));
        scene("heat").unwrap();
        for name in OUTPUT_CHANNELS {
            let error = scene(name).unwrap_err().to_string();
            assert!(error.contains("reserved"), "{name}: {error}");
        }
    }
    #[test]
    fn image_handles_reject_sampling_options_for_every_program() {
        let scene = |program: &str, texture: serde_json::Value| {
            parse(serde_json::json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
                "images":{"n":{"file":"n.png"}},"programs":{"p":{"source":"import vork;"}},
                "materials":{"m":{"program":program,"inputs":{"specular_roughness":texture}}}}))
        };
        scene("p", serde_json::json!({"image":"n"})).unwrap();
        scene("std/openpbr", serde_json::json!({"image":"n"})).unwrap();
        for option in [serde_json::json!({"factor":0.5}), serde_json::json!({"channel":"r"}), serde_json::json!({"tex_coord":1}),
            serde_json::json!({"scale":[2,2]}), serde_json::json!({"offset":[0.5,0]}), serde_json::json!({"rotation":1})] {
            let mut texture = option.clone();
            texture["image"] = "n".into();
            for program in ["std/openpbr", "p"] {
                assert!(scene(program, texture.clone()).is_err(), "{program}: {texture}");
            }
        }
    }
    #[test]
    fn removed_program_diagnostics_route_to_upgrade() {
        for program in ["std/fog", "post/vignette", "post/lens_distortion", "post/glare"] {
            // Public builds reject removed programs even in unused definitions.
            if crate::libraries::module_source(program).is_some() { continue; }
            for field in ["materials", "display"] {
                let mut value = serde_json::json!({"version":2,"nodes":[{"type":"camera"}]});
                if field == "materials" {
                    value["materials"] = serde_json::json!({"unused":{"program":program}});
                } else {
                    value["render"] = serde_json::json!({"display":{"program":program}});
                }
                let error = parse(value).unwrap_err().to_string();
                assert!(error.contains("vork help upgrade"), "{program} {field}: {error}");
            }
        }
    }
    #[test]
    fn removed_principled_surface_suggests_openpbr() {
        let error = parse(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
            "materials":{"m":{"program":"blender/principled"}}})).unwrap_err().to_string();
        assert!(error.contains("unknown program \"blender/principled\"")
            && error.contains("use std/openpbr"), "{error}");
        assert!(crate::libraries::module_source("blender/principled").is_none());
        assert_eq!(crate::libraries::module_source("blender/principled_volume").is_some(), cfg!(feature="test-support"));
    }
    #[test]
    fn camera_material_uses_shared_inline_material() {
        let scene=Scene::from_json(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","name":"camera","material":{
                "program":{"source":"struct Inputs {float start=0.;}; CameraRay camera(CameraContext c,Inputs i) {CameraRay r;r.origin=c.origin;r.direction=c.forward;r.t_min=0.;r.t_max=1000.;return r;} float3 sensor(SensorContext c,Inputs i) {return c.path_length>=i.start ? 1.:0.;}"},
                "inputs":{"start":2}}}]}).to_string().as_bytes()).unwrap();
        let NodeKind::Camera {material:Some(name),..}=&scene.nodes[0].kind else {panic!("camera material missing")};
        assert_eq!(scene.materials[name].inputs["start"],Input::Number(2.));
        assert!(parse(serde_json::json!({"version":2,"nodes":[{"type":"camera","material":"missing"}]})).is_err());
        assert!(parse(serde_json::json!({"version":2,"nodes":[{"type":"camera"}],"render":{"path_length_gate":{"start":0,"end":1}}})).is_err());
    }

    #[test]
    fn openpbr_inputs_are_exactly_the_specification_table() {
        assert_eq!(OPENPBR.len(), 37);
        let line = include_str!("reference.txt").lines()
            .find(|l| l.starts_with("material.inputs (std/openpbr): OpenPBR Surface; shaded inputs:")).unwrap();
        let listed = line.split("shaded inputs: ").nth(1).unwrap().split_whitespace().collect::<Vec<_>>();
        assert_eq!(listed, OpenPbrParam::supported().map(|p| p.name).collect::<Vec<_>>());
        for param in OPENPBR {
            let path = format!("material.inputs.{}: ", param.name);
            let records = include_str!("reference.txt").lines().filter(|l| l.starts_with(&path)).collect::<Vec<_>>();
            assert_eq!(records.len(), usize::from(param.supported), "{}", param.name);
            if param.supported && param.kind == OpenPbrKind::Vector3 {
                assert!(records[0].contains("constant or texture; animatable"), "{}", records[0]);
            }
        }
        let p = |n| OpenPbrParam::find(n).unwrap();
        assert_eq!(p("specular_roughness").default, &[0.3]);
        assert_eq!(p("coat_ior").default, &[1.6]);
        assert_eq!(p("transmission_dispersion_scale").default, &[0.]);
        assert_eq!(p("transmission_dispersion_abbe_number").default, &[20.]);
        assert!(!p("transmission_dispersion_abbe_number").allows(0.));
        assert!(!p("specular_ior").allows(0.) && p("specular_ior").allows(4.));
        let scene = |inputs: serde_json::Value| {
            parse(serde_json::json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
                "images":{"n":{"file":"n.png"}},"materials":{"m":{"inputs":inputs}}}))
        };
        for valid in [
            serde_json::json!({"geometry_coat_normal":{"image":"n"}}),
            serde_json::json!({"fuzz_weight":1,"base_color":[1,0,0],"geometry_thin_walled":true}),
            serde_json::json!({"geometry_normal":{"image":"n"},"emission_luminance":{"image":"n"}}),
        ] {
            scene(valid.clone()).unwrap_or_else(|e| panic!("{valid}: {e}"));
        }
        for invalid in [
            serde_json::json!({"sheen_color":[1,1,1]}),
            serde_json::json!({"geometry_thickness":0.1}),
            serde_json::json!({"specular_anisotropy":0.5}),
            serde_json::json!({"base_color":0.5}),
            serde_json::json!({"base_metalness":2}),
            serde_json::json!({"geometry_thin_walled":1}),
            serde_json::json!({"base_color":"missing"}),
        ] {
            assert!(scene(invalid.clone()).is_err(), "accepted {invalid}");
        }
        for (wrong, meant) in [
            ("roughness", "specular_roughness"),
            ("metallic", "base_metalness"),
            ("sheen_color", "fuzz_color"),
        ] {
            let error = format!("{:#}", scene(serde_json::json!({wrong: 0.5})).unwrap_err());
            assert!(
                error.contains(&format!("did you mean {meant:?}?")),
                "{error}"
            );
        }
    }
}

#[cfg(test)]
mod standard_defaults_tests {
    use super::*;
    #[test]
    fn defaults_match_blender_521_factory_and_add_object_receipts() {
        let b: serde_json::Value = crate::v2::from_test_str(include_str!(
            "../../tests/fixtures/blender-5.2.1-defaults.json"
        ))
        .unwrap();
        let lights: serde_json::Value = crate::v2::from_test_str(include_str!(
            "../../tests/fixtures/blender-5.2.1-light-objects.json"
        ))
        .unwrap();
        let light: Node =
            crate::v2::from_test_value(serde_json::json!({"type":"light","shape":"rectangle"})).unwrap();
        let l = serde_json::to_value(light).unwrap();
        assert_eq!(l["size"], lights["AREA"]["size"]);
        assert!(l.get("shadow").is_none());
        assert_eq!(
            LightShape::Sun.default_energy(),
            lights["SUN"]["energy"].as_f64().unwrap() as f32
        );
        assert_eq!(
            LightShape::Point.default_energy(),
            lights["POINT"]["energy"].as_f64().unwrap() as f32
        );
        for (shape, source) in [("point", "POINT"), ("spot", "SPOT")] {
            let light = serde_json::to_value(crate::v2::from_test_value::<Node>(serde_json::json!({"type":"light","shape":shape})).unwrap()).unwrap();
            assert_eq!(light["radius"], lights[source]["shadow_soft_size"]);
        }
        let spot = serde_json::to_value(crate::v2::from_test_value::<Node>(serde_json::json!({"type":"light","shape":"spot"})).unwrap()).unwrap();
        assert_eq!(LightShape::Spot.default_energy(), lights["SPOT"]["energy"].as_f64().unwrap() as f32);
        assert_eq!(spot["spot_size"].as_f64().unwrap() as f32, b["lights"]["SPOT"]["spot_size"].as_f64().unwrap() as f32);
        assert_eq!(spot["spot_blend"].as_f64().unwrap() as f32, b["lights"]["SPOT"]["spot_blend"].as_f64().unwrap() as f32);
        let r = Render::default();
        assert_eq!(r.fps.as_f32(), b["render"]["fps"].as_f64().unwrap() as f32);
        assert_eq!(r.duration.as_f32(), 250. / r.fps.as_f32());
        assert_eq!((r.duration.exact()*vork_time::exact::decimal(&r.fps.to_string()).unwrap()).ceil().to_integer(),250.into());

        assert_eq!(
            constants::ENVIRONMENT_COLOR[0],
            b["world_background"]["Color"][0].as_f64().unwrap() as f32
        );
    }
}

pub use constants::{
    ENVIRONMENT_COLOR, LIGHT_TEMPERATURE_MAX, LIGHT_TEMPERATURE_MIN, MAX_RENDER_SAMPLES, OUTPUT_CHANNELS,
    RENDER_SAMPLES, MOTION_BLUR_SHUTTER,
};

#[cfg(test)]
mod light_link_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn all_groups_and_link_names_are_checked_before_install() {
        let mut value = json!({"version":2,"nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}},{"type":"node","light_link_tags":(0..130).map(|i|format!("g{i}")).collect::<Vec<_>>()},{"type":"asset","file":"unused.glb","nodes":{"Wheel":{"light_link_tags":["extra"]}}}]});
        let scene: Scene = crate::v2::from_test_value(value.clone()).unwrap();
        scene.validate().unwrap();
        value["nodes"][2] = json!({"type":"light","shape":"point","illuminates":["missing"]});
        let error = crate::v2::from_test_value::<Scene>(value)
            .unwrap()
            .validate()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("light_link_tags entry") && error.contains("valid names"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod launch_api_tests {
    #[test]
    fn world_medium_is_a_public_inline_material_field() {
        let scene=super::Scene::from_json(br#"{"version":2,"nodes":[{"type":"camera"},{"type":"light","shape":"environment","medium":{"program":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c, Inputs i) { return MediumCoefficients(); }"}}}]}"#).unwrap();
        assert!(matches!(&scene.nodes[1].kind,super::NodeKind::Light {medium:Some(_),..}));
        let schema=super::schema().to_value().to_string();
        assert!(schema.contains("light.medium"));
        assert!(schema.contains("\"medium\":"));
        assert!(!schema.contains("\"volume\""));
    }
}

#[cfg(test)]
mod geometry_stage_tests {
    use super::*;
    #[test]
    fn stage_list_default_order_roundtrip_and_errors() {
        let empty=serde_json::json!({"version":2,"nodes":[{"type":"camera"}]});
        let baseline=Scene::from_json(&serde_json::to_vec(&empty).unwrap()).unwrap();
        assert_eq!(baseline.geometry_stages,GeometryStage::DEFAULT);
        for names in [vec!["deform","compute","displace"],vec!["displace","deform","compute"],vec![]] {
            let mut value=empty.clone();value["geometry_stages"]=serde_json::json!(names);
            let scene=Scene::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(serde_json::to_value(scene).unwrap()["geometry_stages"],value["geometry_stages"]);
        }
        let mut value=empty.clone();value["geometry_stages"]=serde_json::json!(["compute","compute"]);
        let error=format!("{:#}",Scene::from_json(&serde_json::to_vec(&value).unwrap()).unwrap_err());
        assert!(error.contains("geometry_stages[1]") && error.contains("remove the duplicate"),"{error}");
        value["geometry_stages"]=serde_json::json!(["skin"]);
        let error=format!("{:#}",Scene::from_json(&serde_json::to_vec(&value).unwrap()).unwrap_err());
        assert!(error.contains("deform") && error.contains("compute") && error.contains("displace"),"{error}");
    }
}

#[cfg(test)]
pub(crate) const COPY_COMPOSITOR: &str = "struct Inputs {Image original;Image source;Image texture;}; float4 composite(CompositeContext c,Inputs i){return compositor_sample(c.uv);}";
