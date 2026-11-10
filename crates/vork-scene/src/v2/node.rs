use super::*;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Node {
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.name", "x-help" = "string; globally unique, including imported nodes"))]
    pub name: String,
    /// glTF node TRS, written directly on the node (`translation`, `rotation`, `scale`).
    #[serde(default, flatten)]
    pub transform: Transform,
    #[schemars(extend("x-source" = "Blender Track To constraint (vork target shorthand)", "x-help-path" = "node.target", "x-help" = "node-name|[x,y,z]|null; aims -Z, world Y up; rotation is an offset on top; structural, not animatable: aim at a named node and animate its translation"))]
    pub target: Option<AimTarget>,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.children", "x-help" = "node[]"))]
    pub children: Vec<Node>,
    #[serde(default, flatten)]
    pub visibility: Visibility,
    #[serde(default)]
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.light_link_tags", "x-help" = "string[]; matched by light illuminates/shadowed_by; inherited"))]
    pub light_link_tags: Vec<String>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.pass_index", "x-help" = "u32|null; 1..65535; object ID"))]
    pub pass_index: Option<u32>,
    /// Cycles holdout: transparent black on transparent film; opaque black otherwise.
    #[serde(default)]
    #[schemars(extend("x-source" = "Blender Cycles Object.is_holdout", "x-help-path" = "node.is_holdout", "x-help" = "bool; RGB 0; alpha 0 on transparent background, 1 on opaque background"))]
    pub is_holdout: bool,
    /// Cycles shadow catcher. Combined treats this surface as a holdout.
    #[serde(default)]
    #[schemars(extend("x-source" = "Blender Cycles Object.is_shadow_catcher", "x-help-path" = "node.is_shadow_catcher", "x-help" = "bool; shadow_catcher channel is material light with CG / without CG, unclamped; zero baseline gives neutral 1; CG light on an otherwise unlit catcher cannot be represented"))]
    pub is_shadow_catcher: bool,
    #[serde(flatten)]
    pub kind: NodeKind,
}
// Keep the strict derived variant fields; omission alone selects a plain group.
impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        // Node-kind values are numeric program inputs, not scene clocks.
        super::lower_numeric_json(&mut value).map_err(serde::de::Error::custom)?;
        if let Some(object) = value.as_object_mut() {
            object.entry("type").or_insert_with(|| serde_json::json!("node"));
        }
        NodeKind::deserialize(value).map_err(serde::de::Error::custom)
    }
}
impl Serialize for NodeKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        NodeKind::serialize(self, serializer)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Visibility is ANDed with ancestors. Reflections and diffuse transport query
/// scene geometry independently of camera visibility. Diffuse visibility controls AO
/// and indirect occlusion; specular visibility controls reflected appearance.
pub struct Visibility {
    #[schemars(extend("x-source" = "vork visibility primitive", "x-help-path" = "node.visible", "x-help" = "bool; false hides the subtree and disables its lights; animatable (step)"))]
    pub visible: bool,
    #[schemars(extend("x-source" = "Blender Object.visible_camera", "x-help-path" = "node.visible_camera", "x-help" = "bool"))]
    #[serde(rename = "visible_camera")]
    pub camera: bool,
    #[schemars(extend("x-source" = "Blender Object.visible_shadow", "x-help-path" = "node.visible_shadow", "x-help" = "bool; lights must stay true (a light without shadows: shadowed_by [])"))]
    #[serde(rename = "visible_shadow")]
    pub shadow: bool,
    #[schemars(extend("x-source" = "Blender Object.visible_diffuse", "x-help-path" = "node.visible_diffuse", "x-help" = "bool"))]
    #[serde(rename = "visible_diffuse")]
    pub diffuse: bool,
    #[schemars(extend("x-source" = "Blender Object.visible_glossy", "x-help-path" = "node.visible_glossy", "x-help" = "bool"))]
    #[serde(rename = "visible_glossy")]
    pub glossy: bool,
    #[schemars(extend("x-source" = "Blender Object.visible_transmission", "x-help-path" = "node.visible_transmission", "x-help" = "bool; object visibility to transmission rays; lights must stay true"))]
    #[serde(rename = "visible_transmission")]
    pub transmission: bool,
}
impl Default for Visibility {
    fn default() -> Self {
        Self {
            visible: true,
            camera: true,
            shadow: true,
            diffuse: true,
            glossy: true,
            transmission: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(remote = "Self", tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(extend("x-help-path" = "node", "x-help-extra" = "node.type: node|mesh|asset|camera|light; default \"node\"\nasset (glTF cameras): addressed as <asset>/<node-name|#index>"))]
pub enum NodeKind {
    #[schemars(title = "TransformNode")]
    Node {},
    #[schemars(title = "MeshNode")]
    Mesh {
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.geometry", "x-help" = "geometry-name|inline geometry"))]
        #[schemars(with = "NameOr<Geometry>")]
        geometry: String,
        #[serde(default)]
        #[schemars(extend("x-source" = "OpenPBR Surface 1.1.1", "x-help-path" = "node.mesh.materials", "x-help" = "(material-name|inline material)[]; empty = default std/openpbr with roughness 0.3"))]
        #[schemars(with = "Vec<NameOr<Material>>")]
        materials: Vec<String>,
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.instances", "x-help" = "instances|null"))]
        instances: Option<Instances>,
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.skin", "x-help" = "skin|null"))]
        skin: Option<Skin>,
        #[serde(default)]
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.mesh.morph_weights", "x-help" = "{name:number}"))]
        morph_weights: Table<f32>,
    },
    #[schemars(title = "AssetNode")]
    Asset {
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.asset.file", "x-help" = "file-path"))]
        file: String,
        #[serde(default)]
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.asset.materials", "x-help" = "{imported-name|#index|*: material-name|inline material}; #index = glTF material index; specific beats *"))]
        #[schemars(with = "Table<NameOr<Material>>")]
        materials: Table<String>,
        #[serde(default)]
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.asset.nodes", "x-help" = "{imported-name|#index: overrides}; #index = glTF node index"))]
        nodes: Table<AssetNodeOverrides>,
    },
    #[schemars(title = "CameraNode")]
    Camera {
        #[serde(default)]
        #[schemars(extend("x-source" = "vork (programmable camera and sensor)", "x-help-path" = "camera.material", "x-help" = "material-name|inline material|null; omitted = std/camera; optics are program inputs"))]
        #[schemars(with = "Option<NameOr<Material>>")]
        material: Option<String>,
        #[serde(default)]
        #[schemars(extend("x-source" = "Blender CameraDOFSettings.focus_object", "x-help-path" = "camera.focus_object", "x-help" = "node-name|null; camera-axis depth supplied to the program at sample time"))]
        focus_object: Option<String>,
    },
    #[schemars(title = "LightNode")]
    Light {
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "light.shape", "x-help" = "sun|point|spot|rectangle|ellipse|environment"))]
        shape: LightShape,
        /// None: LightShape::default_color (white, or Blender's world colour for an
        /// environment without an image).
        #[schemars(extend("x-source" = "Blender 5.2.1 Light.color (Add Light default)", "x-help-path" = "light.color", "x-help" = "RGB"))]
        color: Option<[f32; 3]>,
        /// None: `color` alone. Some(kelvin): `color` × std.color blackbody(kelvin).
        #[schemars(extend("x-source" = "Blender 5.2.1 Light.temperature with use_temperature", "x-help-path" = "light.temperature", "x-help" = "number|null; K; multiplies color; not environment"))]
        temperature: Option<f32>,
        #[schemars(extend("x-source" = "Blender 5.2.1 Light.energy (Add Light default)", "x-help-path" = "light.energy", "x-help" = "number|null; sun W/m²; local W; environment multiplier"))]
        energy: Option<f32>,
        #[serde(default = "constants::sun_angle")]
        #[schemars(extend("x-source" = "Blender 5.2.1 SunLight.angle (Add Light default)", "x-help-path" = "light.angle", "x-help" = "number; sun"))]
        angle: f32,
        #[serde(default)]
        #[schemars(extend("x-source" = "Blender 5.2.1 Light.shadow_soft_size (Add Light default)", "x-help-path" = "light.radius", "x-help" = "number; point/spot emitter radius (shadow softness)"))]
        radius: f32,
        #[serde(default = "constants::spot_size")]
        #[schemars(extend("x-source" = "Blender 5.2.1 SpotLight.spot_size", "x-help-path" = "light.spot_size", "x-help" = "number; full cone angle"))]
        spot_size: f32,
        #[serde(default = "constants::spot_blend")]
        #[schemars(extend("x-source" = "Blender 5.2.1 SpotLight.spot_blend", "x-help-path" = "light.spot_blend", "x-help" = "number; 0..1"))]
        spot_blend: f32,
        #[serde(default = "constants::area_dimension")]
        #[schemars(extend("x-source" = "Blender 5.2.1 AreaLight.size (Add Light default)", "x-help-path" = "light.size", "x-help" = "number; rectangle full side, ellipse diameter"))]
        size: f32,
        #[schemars(extend("x-source" = "Blender 5.2.1 AreaLight square/disk sizing", "x-help-path" = "light.size_y", "x-help" = "number|null; second side/diameter; omitted = size (square/disk)"))]
        size_y: Option<f32>,
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "light.image", "x-help" = "image-name|inline image|null; LM-63 Type C IES (path only); non-environment lights; emits -Z, no peak normalization"))]
        #[schemars(with = "Option<NameOr<Image>>")]
        image: Option<String>,
        /// Physical world fog material on an environment light.
        #[schemars(extend("x-source" = "vork (physical medium)", "x-help-path" = "light.medium", "x-help" = "material-name|inline material|null; environment only; authored medium program for world fog"))]
        #[schemars(with = "Option<NameOr<Material>>")]
        medium: Option<String>,
        /// Environment radiance material instance.
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "light.material", "x-help" = "material-name|inline material|null; environment program"))]
        #[schemars(with = "Option<NameOr<Material>>")]
        material: Option<String>,
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "light.illuminates", "x-help" = "light-link-tag[]|null; receivers with any tag; null=all; []=none"))]
        illuminates: Option<Vec<String>>,
        #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "light.shadowed_by", "x-help" = "light-link-tag[]|null; casters with any tag; null=all; []=none"))]
        shadowed_by: Option<Vec<String>>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("x-help-extra" = "node.asset.nodes.<name>.visible: bool; default true\nnode.asset.nodes.<name>.visible_camera: bool; default true\nnode.asset.nodes.<name>.visible_shadow: bool; default true\nnode.asset.nodes.<name>.visible_diffuse: bool; default true\nnode.asset.nodes.<name>.visible_glossy: bool; default true\nnode.asset.nodes.<name>.visible_transmission: bool; default true"))]
pub struct AssetNodeOverrides {
    #[serde(default, flatten)]
    pub visibility: Option<Visibility>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.asset.nodes.<name>.light_link_tags", "x-help" = "string[]|null; replaces inherited tags"))]
    pub light_link_tags: Option<Vec<String>>,
    #[schemars(extend("x-source" = "vork (explicit contract/technique choice)", "x-help-path" = "node.asset.nodes.<name>.pass_index", "x-help" = "u32|null; 1..65535; object ID"))]
    pub pass_index: Option<u32>,
}
/// Aiming references a node origin or a fixed point in world coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AimTarget {
    Node(String),
    Point(#[serde(deserialize_with="super::numeric")] [f32; 3]),
}
/// Translation, rotation and scale (glTF node TRS), flat on the node. Rotation is
/// stored as a quaternion; an authored `{"euler": [x,y,z]}` converts losslessly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Transform {
    #[schemars(extend("x-source" = "glTF 2.0", "x-help-path" = "node.translation", "x-help" = "[x,y,z]; parent frame"))]
    #[serde(deserialize_with="super::numeric")]
    pub translation: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "authored_rotation")]
    #[schemars(with = "Rotation", extend("default" = [0., 0., 0., 1.]))]
    #[schemars(extend("x-source" = "glTF 2.0 (quaternion); Blender XYZ Euler", "x-help-path" = "node.rotation", "x-help" = "[x,y,z,w] quaternion | {euler:[x,y,z]} (Blender XYZ)"))]
    // Preserve omission so targets can reject even an authored identity.
    pub rotation: Option<[f32; 4]>,
    #[schemars(extend("x-source" = "glTF 2.0", "x-help-path" = "node.scale", "x-help" = "[x,y,z]; any zero world-scale component hides geometry, including children"))]
    #[serde(deserialize_with="super::numeric")]
    pub scale: [f32; 3],
}
/// An authored rotation: a glTF quaternion or a tagged Euler triple.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum Rotation {
    Quaternion(#[serde(deserialize_with="super::numeric")] [f32; 4]),
    Euler {
        /// Radians about X, then Y, then Z (fixed parent axes; Blender XYZ).
        #[serde(deserialize_with="super::numeric")]
        euler: [f32; 3],
    },
}
impl Rotation {
    /// XYZW quaternion; Euler XYZ is R = Rz * Ry * Rx.
    pub fn quaternion(self) -> [f32; 4] {
        match self {
            Self::Quaternion(q) => q,
            Self::Euler { euler: [x, y, z] } => (glam::Quat::from_rotation_z(z)
                * glam::Quat::from_rotation_y(y)
                * glam::Quat::from_rotation_x(x)).to_array(),
        }
    }
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.; 3],
            rotation: None,
            scale: [1.; 3],
        }
    }
}
fn authored_rotation<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<[f32; 4]>, D::Error> {
    Rotation::deserialize(deserializer).map(|r| Some(r.quaternion())).map_err(|_| serde::de::Error::custom(
        "rotation must be [x,y,z,w] (unit quaternion) or {\"euler\": [x,y,z]} (radians, X then Y then Z)"))
}
impl Transform {
    pub fn resolved_rotation(&self) -> [f32; 4] {
        self.rotation.unwrap_or([0., 0., 0., 1.])
    }
    pub fn rotation_mut(&mut self) -> &mut [f32; 4] {
        self.rotation.get_or_insert([0., 0., 0., 1.])
    }
    pub fn matrix(&self) -> glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(
            self.scale.into(),
            glam::Quat::from_array(self.resolved_rotation()).normalize(),
            self.translation.into(),
        )
    }
}
/// Testing aid: replace a node document's whole transform (`translation`, `rotation`, `scale`).
#[doc(hidden)]
pub fn set_transform(node: &mut serde_json::Value, trs: serde_json::Value) {
    let Some(node) = node.as_object_mut() else { return };
    for key in ["translation", "rotation", "scale"] { node.remove(key); }
    node.extend(trs.as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
}
impl NodeKind {
    /// Whether this light permits shadow casters.
    pub fn casts_shadows(&self) -> bool {
        matches!(self, Self::Light { shadowed_by, .. } if !shadowed_by.as_ref().is_some_and(Vec::is_empty))
    }
}
impl Node {
    /// This node without its subtree (the renderer flattens the tree once).
    pub fn without_children(&self) -> Self {
        Self {
            name: self.name.clone(),
            transform: self.transform.clone(),
            target: self.target.clone(),
            children: Vec::new(),
            visibility: self.visibility.clone(),
            light_link_tags: self.light_link_tags.clone(),
            pass_index: self.pass_index,
            is_holdout: self.is_holdout, is_shadow_catcher: self.is_shadow_catcher,
            kind: self.kind.clone(),
        }
    }
}


#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[test]
    fn omitted_type_is_a_strict_group_node() {
        let node: Node = crate::v2::from_test_value(serde_json::json!({"name":"group","children":[{}]})).unwrap();
        assert!(matches!(node.kind, NodeKind::Node {}));
        assert!(matches!(node.children[0].kind, NodeKind::Node {}));
        for value in [serde_json::json!({"geometry":"g"}), serde_json::json!({"lens":50}),
            serde_json::json!({"energy":10}), serde_json::json!({"file":"a.glb"}),
            serde_json::json!({"typo":true}), serde_json::json!({"type":"meshh"}),
            serde_json::json!({"type":null}), serde_json::json!({"type":1})] {
            assert!(crate::v2::from_test_value::<Node>(value.clone()).is_err(), "{value}");
        }
        assert_eq!(serde_json::to_value(node).unwrap()["type"], "node");
        let scene = Scene::from_json(br#"{"version":2,"nodes":[{"name":"group","children":[{"type":"camera","name":"camera"}]}]}"#).unwrap();
        assert!(matches!(scene.nodes[0].kind, NodeKind::Node {}));
    }

    #[test]
    fn flat_rna_visibility_round_trips_on_nodes_and_imported_overrides() {
        let value = serde_json::json!({"version":2,"nodes":[{
            "type":"asset","file":"model.glb","visible_camera":false,
            "nodes":{"Wheel":{"visible_shadow":false}}
        }]});
        let scene: Scene = crate::v2::from_test_value(value).unwrap();
        assert!(!scene.nodes[0].visibility.camera);
        assert!(scene.nodes[0].visibility.shadow);
        let NodeKind::Asset { nodes, .. } = &scene.nodes[0].kind else { panic!() };
        let visibility = nodes["Wheel"].visibility.as_ref().unwrap();
        assert!(!visibility.shadow);
        assert!(visibility.camera);
        let value = serde_json::to_value(scene).unwrap();
        assert_eq!(value["nodes"][0]["visible_camera"], false);
        assert_eq!(value["nodes"][0]["nodes"]["Wheel"]["visible_shadow"], false);
        assert!(value["nodes"][0].get("visibility").is_none());
        assert!(crate::v2::from_test_value::<Scene>(serde_json::json!({"version":2,"nodes":[{
            "type":"node","visibility":{"camera":false}
        }]})).is_err());
    }
}

#[cfg(test)]
mod fit_defaults_tests {
    use super::*;
    #[test]
    fn omitted_area_height_stays_linked_to_size() {
        let scene = Scene::from_json(br#"{"version":2,"nodes":[{"type":"light","shape":"rectangle","size":4},{"type":"camera"}]}"#).unwrap();
        let NodeKind::Light { size, size_y, .. } = &scene.nodes[0].kind else { panic!() };
        assert_eq!(*size, 4.);
        assert_eq!(*size_y, None);
        assert_eq!(size_y.unwrap_or(*size), 4.);
    }
}
