//! Track paths: arrays of segments resolved once against the scene into typed targets.
//! Validation and the renderer share this one table of keyable properties.
use super::*;
use anyhow::Context;

/// What a track animates.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackTarget {
    /// A property of an authored node.
    Node {
        node: String,
        property: NodeProperty,
    },
    /// A property of a node imported by asset node `asset`: its glTF name or `#index`.
    Imported {
        asset: String,
        node: String,
        property: NodeProperty,
    },
    /// An input of a material.
    Material {
        material: String,
        input: String,
    },
    /// The weight of a named animation layer.
    LayerWeight {
        layer: String,
    },
    ImageTime { image: String },
    Render(RenderProperty),
}
#[derive(Clone, Debug, PartialEq)]
pub enum NodeProperty {
    Translation,
    Rotation,
    Scale,
    Visibility(Ray),
    MorphWeight(String),
    Color,
    Temperature,
    Energy,
    Angle,
    Radius,
    SpotBlend,
    SpotSize,
    Width,
    Height,
}
/// A node visibility flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ray {
    All,
    Camera,
    Shadow,
    Diffuse,
    Glossy,
    Transmission,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderProperty {
    Camera,
    Background,
    MotionBlurShutter,
}

/// Node property segments: (segments, property name for errors).
const NODE_PROPERTIES: &[&str] = &[
    "visible",
    "translation",
    "rotation",
    "scale",
    "visible_camera",
    "visible_shadow",
    "visible_diffuse",
    "visible_glossy",
    "visible_transmission",
    "morph_weights",
    "color",
    "temperature",
    "energy",
    "angle",
    "radius",
    "spot_size",
    "spot_blend",
    "size",
    "size_y",
];
const RAYS: &[(&str, Ray)] = &[
    ("visible", Ray::All),
    ("visible_camera", Ray::Camera),
    ("visible_shadow", Ray::Shadow),
    ("visible_diffuse", Ray::Diffuse),
    ("visible_glossy", Ray::Glossy),
    ("visible_transmission", Ray::Transmission),
];

impl NodeProperty {
    /// The path segments naming this property.
    fn segments(&self) -> Vec<String> {
        use NodeProperty as P;
        let one = |s: &str| vec![s.to_string()];
        match self {
            P::Translation => one("translation"),
            P::Rotation => one("rotation"),
            P::Scale => one("scale"),
            P::Visibility(ray) => one(RAYS.iter().find(|(_, r)| r == ray).unwrap().0),
            P::MorphWeight(name) => vec!["morph_weights".into(), name.clone()],
            P::Color => one("color"),
            P::Temperature => one("temperature"),
            P::Energy => one("energy"),
            P::Angle => one("angle"),
            P::Radius => one("radius"),
            P::SpotBlend => one("spot_blend"),
            P::SpotSize => one("spot_size"),
            P::Width => one("size"),
            P::Height => one("size_y"),
        }
    }
}
/// A resolved target prints as its track path, e.g. ["nodes","cam","rotation"].
impl std::fmt::Display for TrackTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut path: Vec<String> = match self {
            Self::Node { node, property } => {
                let mut p = vec!["nodes".into(), node.clone()];
                p.extend(property.segments());
                p
            }
            Self::Imported {
                asset,
                node,
                property,
            } => {
                let mut p = vec!["nodes".into(), asset.clone(), "nodes".into(), node.clone()];
                p.extend(property.segments());
                p
            }
            Self::Material { material, input } => {
                vec![
                    "materials".into(),
                    material.clone(),
                    "inputs".into(),
                    input.clone(),
                ]
            }
            Self::ImageTime { image } => vec!["images".into(), image.clone(), "time".into()],
            Self::LayerWeight { layer } => vec!["animation".into(), layer.clone(), "weight".into()],
            Self::Render(_) => vec!["render".into()],
        };
        if let Self::Render(property) = self {
            use RenderProperty as R;
            path.extend(
                match property {
                    R::Camera => &["camera"][..],
                    R::Background => &["background"],
                    R::MotionBlurShutter => &["motion_blur", "shutter"],
                }
                .iter()
                .map(|s| s.to_string()),
            );
        }
        f.write_str(&display_path(&path))
    }
}

/// A track path as JSON, for error messages.
pub fn display_path(path: &[String]) -> String {
    serde_json::to_string(path).unwrap_or_default()
}

/// Segment `index` of `path` or an error naming what it should be.
fn segment<'a>(path: &'a [String], index: usize, what: &str) -> Result<&'a str> {
    path.get(index).map(String::as_str).with_context(|| {
        format!(
            "track {}: segment {index} is missing; expected {what}",
            display_path(path)
        )
    })
}
fn end(path: &[String], length: usize) -> Result<()> {
    ensure!(
        path.len() == length,
        "track {}: segment {length} {:?} is past the end of the property",
        display_path(path),
        path.get(length).map(String::as_str).unwrap_or_default()
    );
    Ok(())
}
fn bad_segment<'a>(
    path: &[String],
    index: usize,
    kind: &str,
    valid: impl IntoIterator<Item = &'a str>,
) -> anyhow::Error {
    anyhow::anyhow!(
        "track {}: segment {index}: {}",
        display_path(path),
        diagnostics::unknown(kind, &path[index], valid)
    )
}

impl Scene {
    /// Every track's target, per layer in track order (resolved once).
    pub fn track_targets(&self) -> Result<Vec<Vec<TrackTarget>>> {
        let nodes = self.nodes_by_name();
        self.animation
            .iter()
            .map(|layer| {
                layer
                    .tracks
                    .iter()
                    .flatten()
                    .map(|track| self.resolve_target(&track.path, &nodes))
                    .collect()
            })
            .collect()
    }
    fn nodes_by_name(&self) -> BTreeMap<&str, &Node> {
        fn visit<'a>(nodes: &'a [Node], out: &mut BTreeMap<&'a str, &'a Node>) {
            for node in nodes {
                if !node.name.is_empty() {
                    out.insert(&node.name, node);
                }
                visit(&node.children, out);
            }
        }
        let mut out = BTreeMap::new();
        visit(&self.nodes, &mut out);
        out
    }
    fn resolve_target(
        &self,
        path: &[String],
        nodes: &BTreeMap<&str, &Node>,
    ) -> Result<TrackTarget> {
        let area = segment(path, 0, "nodes, materials, animation or render")?;
        match area {
            "nodes" => {
                let name = segment(path, 1, "a node name")?;
                let node = nodes
                    .get(name)
                    .copied()
                    .ok_or_else(|| bad_segment(path, 1, "node", nodes.keys().copied()))?;
                if let NodeKind::Asset { .. } = &node.kind
                    && path.get(2).map(String::as_str) == Some("nodes")
                {
                    // An imported node, as in node.nodes: its glTF name (may contain
                    // '.' or match a property name) or #index, a positional selector.
                    segment(path, 3, "an imported node name or #index")?;
                    let property = node_property(path, 4)?;
                    ensure!(
                        matches!(
                            property,
                            NodeProperty::Translation
                                | NodeProperty::Rotation
                                | NodeProperty::Scale
                                | NodeProperty::MorphWeight(_)
                                | NodeProperty::Visibility(Ray::All)
                        ),
                        "track {}: imported nodes key translation, rotation, scale, morph_weights and visible",
                        display_path(path)
                    );
                    return Ok(TrackTarget::Imported {
                        asset: name.into(),
                        node: path[3].clone(),
                        property,
                    });
                }
                let property = node_property(path, 2)?;
                self.check_node_property(path, node, &property)?;
                Ok(TrackTarget::Node {
                    node: name.into(),
                    property,
                })
            }
            "images" => {
                let name = segment(path, 1, "an image name")?;
                ensure!(self.images.contains_key(name), "track {}: unknown image {name}", display_path(path));
                ensure!(segment(path, 2, "time")? == "time", "images key time only");
                end(path, 3)?;
                Ok(TrackTarget::ImageTime { image: name.into() })
            }
            "materials" => {
                let name = segment(path, 1, "a material name")?;
                let material = self.materials.get(name).ok_or_else(|| {
                    bad_segment(
                        path,
                        1,
                        "material",
                        self.materials.keys().map(String::as_str),
                    )
                })?;
                if segment(path, 2, "\"inputs\"")? != "inputs" {
                    return Err(bad_segment(path, 2, "material field", ["inputs"]));
                }
                let input = segment(path, 3, "an input name")?;
                end(path, 4)?;
                anyhow::ensure!(!matches!(material.inputs.get(input), Some(Input::Texture(_))), "track {}: image handles are not numeric inputs; animate a program input instead", display_path(path));
                if material.program == "std/openpbr" {
                    validate::validate_openpbr_input(input)
                        .with_context(|| format!("track {}: segment 3", display_path(path)))?;
                }
                Ok(TrackTarget::Material {
                    material: name.into(),
                    input: input.into(),
                })
            }
            "animation" => {
                let name = segment(path, 1, "a layer name")?;
                let layers = self.animation.iter().filter_map(|l| l.name.as_deref());
                if !self
                    .animation
                    .iter()
                    .any(|l| l.name.as_deref() == Some(name))
                {
                    return Err(bad_segment(path, 1, "animation layer", layers));
                }
                if segment(path, 2, "\"weight\"")? != "weight" {
                    return Err(bad_segment(path, 2, "layer property", ["weight"]));
                }
                end(path, 3)?;
                Ok(TrackTarget::LayerWeight { layer: name.into() })
            }
            "render" => {
                let property = match segment(path, 1, "a render property")? {
                    "camera" => RenderProperty::Camera,
                    "background" => {
                        ensure!(matches!(self.render.background, Background::Color(_)), "background track requires a static RGB background; environment/transparent are structural modes");
                        RenderProperty::Background
                    },
                    "motion_blur" => {
                        ensure!(segment(path, 2, "shutter")? == "shutter", "motion_blur keys shutter");
                        ensure!(self.render.motion_blur.is_some(), "motion_blur is null; set a static motion_blur to key it");
                        end(path, 3)?;
                        return Ok(TrackTarget::Render(RenderProperty::MotionBlurShutter));
                    }
                    _ => {
                        return Err(bad_segment(
                            path,
                            1,
                            "keyable render property",
                            ["camera", "background", "motion_blur"],
                        ));
                    }
                };
                end(path, 2)?;
                Ok(TrackTarget::Render(property))
            }
            _ => Err(bad_segment(
                path,
                0,
                "track area",
                ["nodes", "images", "materials", "animation", "render"],
            )),
        }
    }
    /// Whether `property` exists on this node and has a static value to key from.
    fn check_node_property(
        &self,
        path: &[String],
        node: &Node,
        property: &NodeProperty,
    ) -> Result<()> {
        use NodeProperty as P;
        let applies = match (&node.kind, property) {
            (_, P::Translation | P::Rotation | P::Scale | P::Visibility(_)) => true,
            (NodeKind::Mesh { geometry, .. }, P::MorphWeight(name)) => {
                let morphs = &self.geometry[geometry].morphs;
                ensure!(
                    morphs.contains_key(name),
                    "track {}: segment 3: {}",
                    display_path(path),
                    diagnostics::unknown("morph", name, morphs.keys().map(String::as_str))
                );
                true
            }
            (NodeKind::Light { .. }, P::Color | P::Energy) => true,
            (NodeKind::Light { temperature, .. }, P::Temperature) => {
                ensure!(
                    temperature.is_some(),
                    "track {}: temperature keys a light with a temperature",
                    display_path(path)
                );
                true
            }
            (NodeKind::Light { shape, .. }, P::Angle) => matches!(shape, LightShape::Sun),
            (NodeKind::Light { shape, .. }, P::Radius) => matches!(shape, LightShape::Point | LightShape::Spot),
            (NodeKind::Light { shape, .. }, P::SpotBlend | P::SpotSize) => matches!(shape, LightShape::Spot),
            (NodeKind::Light { shape, .. }, P::Width | P::Height) => {
                matches!(shape, LightShape::Rectangle | LightShape::Ellipse)
            }
            _ => false,
        };
        ensure!(
            applies,
            "track {}: segment 2: {:?} does not apply to node {:?}",
            display_path(path),
            path[2],
            node.name
        );
        Ok(())
    }

    pub(super) fn validate_targets(&self, nodes: &BTreeMap<&str, &Node>) -> Result<()> {
        let camera = |name: &str| validate::validate_camera(name, nodes);
        let mut layers = BTreeMap::new();
        for (index, layer) in self.animation.iter().enumerate() {
            if let Some(name) = &layer.name {
                validate_name(name)?;
                ensure!(
                    layers.insert(name.as_str(), index).is_none(),
                    "duplicate animation layer name {name:?}; layer names must be unique"
                );
            }
        }
        for (layer_index, layer) in self.animation.iter().enumerate() {
            if let Some(clip) = &layer.clip {
                ensure!(
                    clip.name.is_none() || clip.index.is_none(),
                    "clip requires at most one of name or index"
                );
                let target = &clip.target;
                let node = nodes.get(target.as_str()).with_context(|| {
                    diagnostics::unknown("clip target node", target, nodes.keys().copied())
                })?;
                // External clips retarget onto an asset's skin joints by name.
                ensure!(
                    matches!(node.kind, NodeKind::Asset { .. }),
                    "clip target {target:?} must name an asset node"
                );
            }
            for track in layer.tracks.iter().flatten() {
                let target = self.resolve_target(&track.path, nodes)?;
                self.validate_keys(track, &target, layer.blend)?;
                match target {
                    TrackTarget::LayerWeight { layer } => ensure!(
                        layer_index < layers[layer.as_str()],
                        "track {}: layer weight must be keyed from a lower layer (earlier in animation)",
                        display_path(&track.path)
                    ),
                    TrackTarget::Render(RenderProperty::Camera) => {
                        if let Values::Inline(values) = &track.values {
                            for value in values {
                                camera(
                                    value.as_str().context(
                                        "render camera track values must be camera names",
                                    )?,
                                )?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// The value a track target takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackValue {
    /// Numbers with this many components per key.
    Numeric(usize),
    Bool,
    /// A camera node name.
    Camera,
    Token,
}
impl TrackTarget {
    pub fn value(&self) -> TrackValue {
        use NodeProperty as P;
        match self {
            Self::Node { property, .. } | Self::Imported { property, .. } => match property {
                P::Translation | P::Scale | P::Color => TrackValue::Numeric(3),
                P::Rotation => TrackValue::Numeric(4),

                P::Visibility(_) => TrackValue::Bool,
                _ => TrackValue::Numeric(1),
            },
            Self::Material { input, .. } => match OpenPbrParam::find(input).map(|p| p.kind) {
                Some(OpenPbrKind::Bool) => TrackValue::Bool,
                Some(OpenPbrKind::Color3 | OpenPbrKind::Vector3) => TrackValue::Numeric(3),
                _ => TrackValue::Numeric(1),
            },
            Self::ImageTime { .. } | Self::LayerWeight { .. } => TrackValue::Numeric(1),
            Self::Render(RenderProperty::Background) => TrackValue::Numeric(3),
            Self::Render(RenderProperty::Camera) => TrackValue::Camera,
            Self::Render(_) => TrackValue::Numeric(1),
        }
    }
}
impl Scene {
    /// Key times, value shapes and buffers for one track against its target.
    fn validate_keys(&self, track: &Track, target: &TrackTarget, blend: Blend) -> Result<()> {
        let path = display_path(&track.path);
        let buffer = |name: &str, what: &str| -> Result<&Buffer> {
            self.buffers.get(name).with_context(|| {
                format!(
                    "track {path}: {what}: {}",
                    diagnostics::unknown("buffer", name, self.buffers.keys().map(String::as_str))
                )
            })
        };
        let keys = match &track.times {
            Times::Inline(t) => {
                ensure!(
                    !t.is_empty()
                        && t.iter().all(|x| x.is_finite() && *x >= 0.)
                        && t.windows(2).all(|w| w[0] < w[1]),
                    "track {path}: times must be non-empty, finite, >= 0 and strictly increasing"
                );
                Some(t.len())
            }
            Times::Buffer(name) => {
                let b = buffer(name, "times")?;
                ensure!(
                    matches!(b.element_type, BufferType::F32),
                    "track {path}: times buffer {name} must be f32"
                );
                b.len()
            }
        };
        // Program types are reflected at install. Validate the authored numeric
        // shape here; reflection checks its width and storage type before binding.
        let value = if let TrackTarget::Material { material, .. } = target {
            if self.materials[material].program != "std/openpbr" {
                let width = match &track.values {
                    Values::Inline(values) => values.first().and_then(|v| v.as_array()).map_or(1, Vec::len),
                    Values::Buffer(name) => buffer(name, "values")?.element_type.width(),
                };
                ensure!((1..=4).contains(&width), "track {path}: program inputs require scalar or vector numeric keys");
                match &track.values {
                    Values::Inline(values) if values.first().is_some_and(|v| v.is_string()) => TrackValue::Token,
                    Values::Inline(values) if values.first().is_some_and(|v| v.is_boolean()) => TrackValue::Bool,
                    _ => TrackValue::Numeric(width),
                }
            } else { target.value() }
        } else { target.value() };
        let numeric = |v: &serde_json::Value, width: usize| match (v, width) {
            (serde_json::Value::Number(n), 1) => n.as_f64().is_some_and(|n| (n as f32).is_finite()),
            (serde_json::Value::Array(a), _) => {
                a.len() == width
                    && a.iter()
                        .all(|n| n.as_f64().is_some_and(|n| (n as f32).is_finite()))
            }
            _ => false,
        };
        let expected = match value {
            TrackValue::Numeric(1) => "numbers".to_string(),
            TrackValue::Numeric(w) => format!("arrays of {w} numbers"),
            TrackValue::Bool => "booleans".into(),
            TrackValue::Camera => "camera names".into(),
            TrackValue::Token => "enum tokens".into(),
        };
        let rotation = matches!(target, TrackTarget::Node { property: NodeProperty::Rotation, .. }
            | TrackTarget::Imported { property: NodeProperty::Rotation, .. });
        let values = match &track.values {
            Values::Inline(values) => {
                // A rotation track is keyed in one form: quaternions or Euler angles.
                let euler = rotation && values.first().is_some_and(|v| v.get("euler").is_some());
                for (i, v) in values.iter().enumerate() {
                    let ok = match value {
                        TrackValue::Numeric(_) if euler => v.as_object().is_some_and(|o| o.len() == 1)
                            && v.get("euler").is_some_and(|angles| numeric(angles, 3)),
                        TrackValue::Numeric(w) => numeric(v, w),
                        TrackValue::Bool => v.is_boolean(),
                        TrackValue::Camera | TrackValue::Token => v.is_string(),

                    };
                    ensure!(ok, "track {path}: value {i} is {v}; expected {}",
                        if euler { "{\"euler\": [x,y,z]} like the first key".to_string() }
                        else if rotation { format!("{expected}, or {{\"euler\": [x,y,z]}} on every key") } else { expected.clone() });
                }
                Some(values.len())
            }
            Values::Buffer(name) => {
                let b = buffer(name, "values")?;
                let TrackValue::Numeric(width) = value else {
                    anyhow::bail!("track {path}: {expected} must be inline values");
                };
                ensure!(
                    b.element_type.width() == width && !matches!(b.element_type, BufferType::Mat4),
                    "track {path}: values buffer {name} has {} components per key; expected {width}",
                    b.element_type.width()
                );
                b.len()
            }
        };
        if let (Some(keys), Some(values)) = (keys, values) {
            ensure!(
                values == keys * if matches!(track.interpolation, Interpolation::Cubic) { 3 } else { 1 },
                "track {path}: {values} values for {keys} times; CUBICSPLINE requires incoming/value/outgoing triples"
            );
        }
        if !matches!(value, TrackValue::Numeric(_)) {
            ensure!(
                matches!(track.interpolation, Interpolation::Step),
                "track {path}: {expected} require step interpolation"
            );
            ensure!(
                blend != Blend::Add,
                "track {path}: {expected} cannot blend additively"
            );
        }
        Ok(())
    }
}

/// The node property starting at segment `index`, with per-segment errors.
fn node_property(path: &[String], index: usize) -> Result<NodeProperty> {
    use NodeProperty as P;
    let name = segment(path, index, "a node property")?;
    let property = match name {
        "target" => anyhow::bail!("track {}: target is structural; aim at a named node and animate that node's translation instead", display_path(path)),
        "translation" => P::Translation,
        "rotation" => P::Rotation,
        "scale" => P::Scale,
        "visible" => P::Visibility(Ray::All),
        "visible_camera" => P::Visibility(Ray::Camera),
        "visible_shadow" => P::Visibility(Ray::Shadow),
        "visible_diffuse" => P::Visibility(Ray::Diffuse),
        "visible_glossy" => P::Visibility(Ray::Glossy),
        "visible_transmission" => P::Visibility(Ray::Transmission),
        "morph_weights" => {
            let morph = segment(path, index + 1, "a morph name")?;
            end(path, index + 2)?;
            return Ok(P::MorphWeight(morph.into()));
        }
        "spot_size" => P::SpotSize,
        "spot_blend" => P::SpotBlend,
        "color" => P::Color,
        "temperature" => P::Temperature,
        "energy" => P::Energy,
        "angle" => P::Angle,
        "radius" => P::Radius,
        "size" => P::Width,
        "size_y" => P::Height,
        _ => {
            return Err(bad_segment(
                path,
                index,
                "keyable node property",
                NODE_PROPERTIES.iter().copied(),
            ));
        }
    };
    end(path, index + 1)?;
    Ok(property)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene() -> Scene {
        crate::v2::from_test_value(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","name":"cam","material":{"program":"std/camera","inputs":{"projection":"persp"}}},{"type":"asset","name":"actor","file":"a.glb"},
                {"type":"light","name":"key","shape":"spot","spot_size":0.7853981633974483,"spot_blend":0.14999999999999986}],
            "materials":{"paint":{}},
            "animation":[{"name":"walk","tracks":[]}]}))
        .unwrap()
    }
    fn path(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn target_track_explains_animated_aim() {
        let s = scene();
        let error = s.resolve_target(&path(&["nodes", "cam", "target"]), &s.nodes_by_name()).unwrap_err().to_string();
        assert!(error.contains(r#"["nodes","cam","target"]"#), "{error}");
        assert!(error.contains("aim at a named node"), "{error}");
        assert!(error.contains("animate that node's translation"), "{error}");
        assert!(s.resolve_target(&path(&["nodes", "cam", "translation"]), &s.nodes_by_name()).is_ok());
    }
    #[test]
    fn camera_optics_are_not_node_track_properties() {
        let s=scene();
        for property in ["lens","shift","dof","clip_start","clip_end","ortho_scale"] {
            assert!(s.resolve_target(&path(&["nodes","cam",property]),&s.nodes_by_name()).is_err());
        }
    }
    #[test]
    fn post_processing_inputs_resolve_as_material_tracks() {
        let mut s=scene();
        s.programs.insert("post".into(),crate::v2::from_test_value(serde_json::json!({"source":"struct Inputs {float gain=1.;}; float4 composite(CompositeContext c,Inputs i){return float4(i.gain);}"})).unwrap());
        s.materials.insert("grade".into(),crate::v2::from_test_value(serde_json::json!({"program":"post"})).unwrap());
        s.render.compositor.push("grade".into());
        let target=s.resolve_target(&path(&["materials","grade","inputs","gain"]),&s.nodes_by_name()).unwrap();
        assert!(matches!(target, TrackTarget::Material { material, input } if material == "grade" && input == "gain"));
    }
    #[test]
    fn render_settings_are_not_animatable() {
        let mut s = scene();
        s.render.settings.integrator.raster_mut().unwrap().fast_gi = Some(Default::default());
        for segments in [&["render", "settings", "fast_gi", "radius"][..], &["render", "integrator", "fast_gi", "radius"]] {
            let error = format!("{:#}", s.resolve_target(&path(segments), &s.nodes_by_name()).unwrap_err());
            assert!(error.contains("keyable render property"), "{error}");
        }
    }
    #[test]
    fn imported_nodes_use_an_explicit_segment() {
        let s = scene();
        // A glTF node named like a property is unambiguous.
        assert_eq!(
            s.resolve_target(&path(&["nodes", "actor", "nodes", "rotation", "rotation"]), &s.nodes_by_name()).unwrap(),
            TrackTarget::Imported { asset: "actor".into(), node: "rotation".into(), property: NodeProperty::Rotation }
        );
        assert_eq!(
            s.resolve_target(&path(&["nodes", "actor", "nodes", "#3", "scale"]), &s.nodes_by_name()).unwrap().to_string(),
            r##"["nodes","actor","nodes","#3","scale"]"##
        );
        let error = format!("{:#}", s.resolve_target(&path(&["nodes", "actor", "Bone.001", "rotation"]), &s.nodes_by_name()).unwrap_err());
        assert!(error.contains("segment 2"), "{error}");
    }
    #[test]
    fn segments_resolve_to_typed_targets() {
        let s = scene();
        assert_eq!(
            s.resolve_target(&path(&["nodes", "actor", "nodes", "Bone.001", "rotation"]), &s.nodes_by_name())
                .unwrap(),
            TrackTarget::Imported {
                asset: "actor".into(),
                node: "Bone.001".into(),
                property: NodeProperty::Rotation
            }
        );
        assert_eq!(
            s.resolve_target(&path(&["nodes", "actor", "translation"]), &s.nodes_by_name())
                .unwrap(),
            TrackTarget::Node {
                node: "actor".into(),
                property: NodeProperty::Translation
            }
        );
        assert_eq!(
            s.resolve_target(&path(&["nodes", "key", "spot_size"]), &s.nodes_by_name())
                .unwrap(),
            TrackTarget::Node {
                node: "key".into(),
                property: NodeProperty::SpotSize
            }
        );
        assert_eq!(
            s.resolve_target(&path(&["materials", "paint", "inputs", "base_color"]), &s.nodes_by_name())
                .unwrap(),
            TrackTarget::Material {
                material: "paint".into(),
                input: "base_color".into()
            }
        );
        assert_eq!(
            s.resolve_target(&path(&["animation", "walk", "weight"]), &s.nodes_by_name())
                .unwrap(),
            TrackTarget::LayerWeight {
                layer: "walk".into()
            }
        );
        assert_eq!(
            s.resolve_target(&path(&["render", "camera"]), &s.nodes_by_name()).unwrap(),
            TrackTarget::Render(RenderProperty::Camera)
        );
        for p in [
            &["nodes", "actor", "nodes", "Bone.001", "rotation"][..],
            &["nodes", "key", "spot_size"],
            &["materials", "paint", "inputs", "base_color"],
            &["animation", "walk", "weight"],
        ] {
            assert_eq!(
                s.resolve_target(&path(p), &s.nodes_by_name()).unwrap().to_string(),
                display_path(&path(p))
            );
        }
    }
    #[test]
    fn errors_name_the_bad_segment() {
        let s = scene();
        for (bad, expect) in [
            (&["nodes", "cmaera", "rotation"][..], "segment 1"),
            (&["nodes", "cam", "rotaton"][..], "segment 2"),
            (&["nodes", "cam", "energy"][..], "segment 2"),
            (&["nodes", "key", "spot_middl"][..], "segment 2"),
            (&["nodes", "cam", "rotation", "x"][..], "segment 3"),
            (&["materials", "paint", "inputs", "colour"][..], "segment 3"),
            (&["render", "fps"][..], "segment 1"),
            (&["layers"][..], "segment 0"),
        ] {
            let error = format!("{:#}", s.resolve_target(&path(bad), &s.nodes_by_name()).unwrap_err());
            assert!(error.contains(expect), "{bad:?}: {error}");
        }
    }
    #[test]
    fn key_shapes_buffers_and_kinds_fail_validation() {
        let check = |tracks: serde_json::Value, blend: &str, expect: &str| {
            let s: Scene = crate::v2::from_test_value(serde_json::json!({"version":2,
                "nodes":[{"type":"camera","name":"cam","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
                "materials":{"paint":{},"custom":{"program":"p"}},"programs":{"p":{"source":"x.slang"}},
                "buffers":{"t":{"type":"f32","data":[0,1]},"v3":{"type":"vec3","data":[[0,0,0],[1,1,1]]}},
                "animation":[{"blend":blend,"tracks":tracks}]}))
            .unwrap();
            let error = format!("{:#}", s.validate().unwrap_err());
            assert!(error.contains(expect), "{tracks}: {error}");
        };
        check(
            serde_json::json!([{"path":["nodes","cam","rotation"],"keys":[[0,[0,0,1]]]}]),
            "replace",
            "expected arrays of 4 numbers",
        );
        check(
            serde_json::json!([{"path":["nodes","cam","rotation"],"keys":{"times":"t","values":"v3"}}]),
            "replace",
            "expected 4",
        );
        check(
            serde_json::json!([{"path":["nodes","cam","translation"],"keys":{"times":"missing","values":"v3"}}]),
            "replace",
            "unknown buffer",
        );
        check(
            serde_json::json!([{"path":["nodes","cam","translation"],"keys":{"times":"t","values":"nope"}}]),
            "replace",
            "unknown buffer",
        );
        check(
            serde_json::json!([{"path":["nodes","cam","translation"],"keys":[[-1,[0,0,0]],[1,[0,0,0]]]}]),
            "replace",
            ">= 0",
        );
        check(
            serde_json::json!([{"path":["nodes","cam","visible_camera"],"keys":[[0,true]],"interpolation":"STEP"}]),
            "add",
            "additively",
        );
        check(
            serde_json::json!([{"path":["materials","paint","inputs","geometry_thin_walled"],"keys":[[0,1]],"interpolation":"STEP"}]),
            "replace",
            "expected booleans",
        );
        check(
            serde_json::json!([{"path":["materials","custom","inputs","x"],"keys":[[0,"resource"],[1,1]]}]),
            "replace",
            "expected enum tokens",
        );
    }
}
