//! Inline definitions: wherever a scene names a resource (a geometry, material,
//! buffer, image or program), it may write the definition there instead. Loading
//! moves each inline definition into its table under a name derived from its
//! content, so identical definitions share one entry (and one GPU resource).
use serde_json::{Map, Value};

/// A table key for an inline definition; `~` marks the name as generated.
fn content_name(definition: &Value) -> String {
    // FNV-1a over the canonical (key-sorted) JSON text: stable across runs and platforms.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in definition.to_string().bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("~{hash:016x}")
}

/// Replace an inline object at `slot` with a name in `table` (created on demand).
fn hoist(root: &mut Map<String, Value>, table: &str, slot: &mut Value) {
    if !slot.is_object() {
        return;
    }
    let mut definition = slot.take();
    lower_resource(root, table, &mut definition);
    let mut name = content_name(&definition);
    let entries = root.entry(table).or_insert_with(|| Value::Object(Map::new()));
    let Some(entries) = entries.as_object_mut() else { *slot = definition; return };
    // A different definition already under this name (a hash collision) gets a suffix.
    let base = name.clone();
    let mut suffix = 1;
    while entries.get(&name).is_some_and(|existing| *existing != definition) {
        name = format!("{base}.{suffix}");
        suffix += 1;
    }
    entries.entry(name.clone()).or_insert(definition);
    *slot = Value::String(name);
}

fn each<'a>(value: Option<&'a mut Value>) -> impl Iterator<Item = &'a mut Value> {
    let items: Box<dyn Iterator<Item = &'a mut Value>> = match value {
        Some(Value::Array(items)) => Box::new(items.iter_mut()),
        Some(Value::Object(items)) => Box::new(items.values_mut()),
        _ => Box::new(std::iter::empty()),
    };
    items
}

// Each edge declares a schema resource slot. `*` visits array items or opaque
// table values; structural edges recurse without hoisting their object.
fn edges(kind: &str) -> &'static [(&'static str, &'static str, bool)] {
    match kind {
        "scene" => &[("nodes.*","node",false),("render","render",false),("animation.*","layer",false),
            ("geometry.*","geometry",false),("materials.*","materials",false),("images.*","images",false),
            ("buffers.*","buffers",false),("programs.*","programs",false)],
        "node" => &[("children.*","node",false),("geometry","geometry",true),("materials.*","materials",true),
            ("instances.count","buffers",true),("instances.attributes.*","buffers",true),
            ("skin.inverse_bind_matrices","buffers",true),("image","images",true),
            ("medium","materials",true),("material","materials",true)],
        "geometry" => &[("ids","buffers",true),("count","buffers",true),("indices","buffers",true),("curve_vertex_counts","buffers",true),("widths","buffers",true),
            ("attributes.*","buffers",true),("morphs.*.POSITION","buffers",true),("morphs.*.NORMAL","buffers",true)],
        "materials" => &[("program","programs",true),("inputs.*","input",false)],
        "input" => &[("image","images",true),("grid","buffers",true)],
        "images" => &[("render","render",false)],
        "render" => &[("compositor.*","composite",false),
            ("display","materials",true)],
        "programs" => &[("bindings.*","buffers",true)],
        "layer" => &[("tracks.*.keys.times","buffers",true),("tracks.*.keys.values","buffers",true)],
        _ => &[],
    }
}

fn slots(value: &mut Value, path: &str, visit: &mut impl FnMut(&mut Value)) {
    if path.is_empty() { visit(value); return; }
    let (head, tail) = path.split_once('.').unwrap_or((path, ""));
    if head == "*" {
        for value in each(Some(value)) { slots(value, tail, visit); }
    } else if let Some(value) = value.get_mut(head) { slots(value, tail, visit); }
}

fn lower_resource(root: &mut Map<String, Value>, kind: &str, entry: &mut Value) {
    if kind == "composite" {
        if entry.get("material").is_some() {
            hoist(root, "materials", &mut entry["material"]);
        } else { hoist(root, "materials", entry); }
        return;
    }
    for &(path, target, resource) in edges(kind) {
        slots(entry, path, &mut |slot| {
            if resource { hoist(root, target, slot); }
            else { lower_resource(root, target, slot); }
        });
    }
}

/// External file slots on the normalized scene. Relocation works
/// on the normalized scene directly, preserving runtime-only state and avoiding
/// a serialization/copy of potentially large buffer data.
macro_rules! resource_files {
    ($apply:ident, $scene:ident) => {
        $apply!(string, $scene.buffers.values_mut().filter_map(|v| v.file.as_mut()));
        $apply!(string, $scene.images.values_mut().filter_map(|v| v.file.as_mut()));
        $apply!(string, $scene.programs.values_mut().filter_map(|v| v.file.as_mut()));
        $apply!(string, $scene.programs.iter_mut().filter_map(|(name, program)| {
            if program.source.is_some() {
                Some(program.source_origin.get_or_insert_with(|| format!("programs.{name}")))
            } else { program.source_origin.as_mut() }
        }));
        $apply!(string, $scene.animation.iter_mut()
            .filter_map(|v| v.clip.as_mut()).filter_map(|v| v.file.as_mut()));
        $apply!(string, {
            fn files(nodes: &mut [super::Node]) -> Vec<&mut String> {
                let mut result = Vec::new();
                for node in nodes {
                    if let super::NodeKind::Asset { file, .. } = &mut node.kind { result.push(file); }
                    result.extend(files(&mut node.children));
                }
                result
            }
            files(&mut $scene.nodes)
        });
    };
}
pub(super) use resource_files;

/// Lower every inline definition in a scene document to a named table entry.
pub(super) fn lower(scene: &mut Value) {
    let Some(root) = scene.as_object_mut() else { return };
    // Keep authored table names reserved while dependencies are allocated.
    for &(path, target, _) in edges("scene") {
        if edges(target).is_empty() { continue; }
        let field = path.split('.').next().unwrap();
        let Some(mut value) = root.remove(field) else { continue };
        if path.ends_with(".*") {
            // Reserve authored resource names during collision allocation.
            if matches!(target, "geometry" | "materials" | "images" | "buffers" | "programs") {
                root.insert(field.into(), value.clone());
            }
            for entry in each(Some(&mut value)) { lower_resource(root, target, entry); }
        } else { lower_resource(root, target, &mut value); }
        if let Some(generated) = root.remove(field).and_then(|v| match v { Value::Object(entries) => Some(entries), _ => None }) {
            if let Some(entries) = value.as_object_mut() {
                for (name, entry) in generated { entries.entry(name).or_insert(entry); }
            }
        }
        root.insert(field.into(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    #[test]
    fn display_inputs_lower_nested_inline_images() {
        let scene = Scene::from_json(&serde_json::to_vec(&serde_json::json!({
            "version":2,"nodes":[{"type":"camera","name":"camera"}],
            "render":{"display":{"program":"blender/filmic","inputs":{
                "lut":{"image":{"render":{"camera":"camera","compositor":[{
                    "program":"post/white_balance","inputs":{"texture":{"image":{"file":"lut.exr"}}}
                }]}}}
            }}}
        })).unwrap()).unwrap();
        assert_eq!(scene.images.len(), 2);
        assert!(scene.images.values().any(|image| image.file.as_deref() == Some("lut.exr")));
        let value = scene.content_json().unwrap();
        let material = value["render"]["display"].as_str().unwrap();
        let name = value["materials"][material]["inputs"]["lut"]["image"].as_str().unwrap();
        assert!(scene.images[name].render.is_some());
    }
    #[test]
    fn buffer_backed_tracks_accept_inline_buffers() {
        let scene = Scene::from_json(br#"{"version":2,"nodes":[{"type":"camera","name":"camera"}],
            "animation":[{"tracks":[{"path":["nodes","camera","translation"],"keys":{
                "times":{"type":"f32","data":[0,1]},
                "values":{"type":"vec3","data":[[0,0,0],[1,0,0]]}
            }}]}]}"#).unwrap();
        assert_eq!(scene.buffers.len(), 2);
        let track = &scene.animation[0].tracks.as_ref().unwrap()[0];
        let (Times::Buffer(times), Values::Buffer(values)) = (&track.times, &track.values) else { panic!() };
        assert!(scene.buffers.contains_key(times) && scene.buffers.contains_key(values));
    }
    #[test]
    fn inline_render_images_lower_their_compositors_recursively() {
        let image = serde_json::json!({"render":{"camera":"capture","root":"empty",
            "compositor":[{"program":"post/white_balance"}]}});
        for document in [
            serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"capture"},{"type":"node","name":"empty"}],
                "materials":{"surface":{"inputs":{"base_color":{"image":image}}}}}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"capture"},{"type":"node","name":"empty"},
                {"type":"light","shape":"environment","material":{"program":"std/environment_image","inputs":{"image":{"image":image}}}}]}),
            serde_json::json!({"version":2,"nodes":[{"type":"camera","name":"capture"},{"type":"node","name":"empty"}],
                "render":{"compositor":[{"program":"post/white_balance","inputs":{"texture":{"image":image}}}]}}),
        ] {
            let loaded = Scene::from_json(&serde_json::to_vec(&document).unwrap());
            let scene=loaded.unwrap();
            let image = scene.images.values().next().unwrap().render.as_ref().unwrap();
            assert_eq!(image.compositor.len(), 1);
            assert_eq!(scene.materials[image.compositor[0].material()].program, "post/white_balance");
        }
    }
    #[test]
    fn generated_image_names_never_replace_authored_definitions() {
        let inline = serde_json::json!({"file":"b.png"});
        let reserved = super::content_name(&inline);
        let scene = Scene::from_json(&serde_json::to_vec(&serde_json::json!({"version":2,
            "nodes":[{"type":"camera"}],
            "images":{reserved.clone():{"file":"a.png"}},
            "materials":{"a":{"inputs":{"base_color":{"image":reserved}}},
                         "b":{"inputs":{"base_color":{"image":inline}}}}
        })).unwrap()).unwrap();
        let authored = scene.content_json().unwrap();
        let a = authored["materials"]["a"]["inputs"]["base_color"]["image"].as_str().unwrap();
        let b = authored["materials"]["b"]["inputs"]["base_color"]["image"].as_str().unwrap();
        assert_ne!(a, b);
        assert_eq!(authored["images"][a]["file"], "a.png");
        assert_eq!(authored["images"][b]["file"], "b.png");
    }
    #[test]
    fn inline_definitions_lower_to_shared_named_entries() {
        let triangle = serde_json::json!({"attributes": {"POSITION": {"type": "vec3", "data": [[0,0,0],[1,0,0],[0,1,0]]}}});
        let red = serde_json::json!({"inputs": {"base_color": [1, 0, 0]}});
        let text = serde_json::json!({"version": 2, "nodes": [
            {"type": "camera"},
            {"type": "mesh", "geometry": triangle, "materials": [red]},
            {"type": "node", "children": [{"type": "mesh", "geometry": triangle, "materials": [red.clone()]}]},
            {"type": "mesh", "geometry": "named", "materials": [{"inputs": {"base_color": {"image": {"file": "a.png"}}}}]}],
            "geometry": {"named": {"attributes": {"POSITION": {"type": "vec3", "data": [[0,0,0],[0,0,1],[0,1,0]]}}}}
        }).to_string();
        let scene = Scene::from_json(text.as_bytes()).unwrap();
        assert_eq!(scene.geometry.len(), 2, "identical inline geometry is shared");
        assert_eq!(scene.buffers.len(), 2);
        assert_eq!(scene.materials.len(), 2, "identical inline materials are shared");
        assert_eq!(scene.images.len(), 1);
        let NodeKind::Mesh { geometry: a, materials: ma, .. } = &scene.nodes[1].kind else { panic!() };
        let NodeKind::Mesh { geometry: b, materials: mb, .. } = &scene.nodes[2].children[0].kind else { panic!() };
        assert_eq!((a, ma), (b, mb));
        assert!(a.starts_with('~'));
        assert!(scene.geometry["named"].attributes["POSITION"].starts_with('~'));
    }
}
