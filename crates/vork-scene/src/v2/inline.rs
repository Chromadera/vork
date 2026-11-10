//! Inline definitions: wherever a scene names a resource (a geometry, material,
//! buffer, image or program), it may write the definition there instead. Loading
//! moves each inline definition into its table under a name derived from its
//! content, so identical definitions share one entry (and one GPU resource).
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Normalized resource keys retain every authored use, independently of sharing.
pub type Origins = BTreeMap<String, Vec<String>>;

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
fn hoist(root: &mut Map<String, Value>, table: &str, slot: &mut Value, path: &str, origins: &mut Origins) {
    if let Some(name) = slot.as_str() {
        origins.entry(format!("{table}.{name}")).or_default().push(path.into());
        return;
    }
    if !slot.is_object() { return; }
    let mut definition = slot.take();
    lower_resource(root, table, &mut definition, path, origins);
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
    origins.entry(format!("{table}.{name}")).or_default().push(path.into());
    *slot = Value::String(name);
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

fn slots(value: &mut Value, edge: &str, path: &str, visit: &mut impl FnMut(&mut Value, &str)) {
    if edge.is_empty() { visit(value, path); return; }
    let (head, tail) = edge.split_once('.').unwrap_or((edge, ""));
    if head == "*" {
        match value {
            Value::Array(items) => for (i, item) in items.iter_mut().enumerate() {
                slots(item, tail, &format!("{path}[{i}]"), visit);
            },
            Value::Object(items) => for (key, item) in items {
                slots(item, tail, &format!("{path}[{key:?}]"), visit);
            },
            _ => {}
        }
    } else if let Some(value) = value.get_mut(head) {
        slots(value, tail, &format!("{path}.{head}"), visit);
    }
}

fn lower_resource(root: &mut Map<String, Value>, kind: &str, entry: &mut Value, path: &str, origins: &mut Origins) {
    if kind == "node" {
        if let Some(name) = entry.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()) {
            origins.entry(format!("nodes.{name}")).or_default().push(path.into());
            if entry.get("type").and_then(Value::as_str) == Some("camera") {
                origins.entry(format!("cameras.{name}")).or_default().push(path.into());
            }
        }
    }
    if kind == "composite" {
        if entry.get("material").is_some() {
            hoist(root, "materials", &mut entry["material"], &format!("{path}.material"), origins);
        } else { hoist(root, "materials", entry, path, origins); }
        return;
    }
    for &(edge, target, resource) in edges(kind) {
        slots(entry, edge, path, &mut |slot, path| {
            if resource { hoist(root, target, slot, path, origins); }
            else { lower_resource(root, target, slot, path, origins); }
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
pub(super) fn lower(scene: &mut Value) -> Origins {
    let mut origins = Origins::new();
    let Some(root) = scene.as_object_mut() else { return origins };
    let authored: std::collections::BTreeSet<_> = ["materials", "programs", "geometry", "images", "buffers"].into_iter()
        .flat_map(|table| root.get(table).and_then(Value::as_object).into_iter().flatten()
            .map(move |(name, _)| format!("{table}.{name}"))).collect();
    // Keep authored table names reserved while dependencies are allocated.
    for &(path, target, _) in edges("scene") {
        if edges(target).is_empty() { continue; }
        let field = path.split('.').next().unwrap();
        let Some(mut value) = root.remove(field) else { continue };
        if path.ends_with(".*") {
            if matches!(target, "geometry" | "materials" | "images" | "buffers" | "programs") && !value.is_object() {
                root.insert(field.into(), value);
                continue;
            }
            // Reserve authored resource names during collision allocation.
            if matches!(target, "geometry" | "materials" | "images" | "buffers" | "programs") {
                root.insert(field.into(), value.clone());
            }
            if let Some(entries) = value.as_object_mut().filter(|_| matches!(target, "materials" | "programs" | "geometry" | "images" | "buffers")) {
                // Carry the actual key: diagnostic paths use Debug escaping and
                // are not a serialization format for resource identifiers.
                for (key, entry) in entries {
                    if !authored.contains(&format!("{target}.{key}")) { continue; }
                    let path = format!("scene.{field}[{key:?}]");
                    origins.entry(format!("{target}.{key}")).or_default().push(path.clone());
                    lower_resource(root, target, entry, &path, &mut origins);
                }
            } else {
                slots(&mut value, "*", &format!("scene.{field}"), &mut |entry, path| {
                    lower_resource(root, target, entry, path, &mut origins);
                });
            }
        } else { lower_resource(root, target, &mut value, &format!("scene.{field}"), &mut origins); }
        if let Some(generated) = root.remove(field).and_then(|v| match v { Value::Object(entries) => Some(entries), _ => None }) {
            if let Some(entries) = value.as_object_mut() {
                for (name, entry) in generated { entries.entry(name).or_insert(entry); }
            }
        }
        root.insert(field.into(), value);
    }
    for paths in origins.values_mut() { paths.sort(); paths.dedup(); }
    origins
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

/// Recover positions from the original JSON, never from its sorted/serialized form.
/// The input has already passed serde's syntax check. Only requested paths are retained.
pub(super) fn locate(bytes: &[u8], origins: &mut Origins) {
    fn ws(bytes: &[u8], pos: &mut usize) {
        while bytes.get(*pos).is_some_and(u8::is_ascii_whitespace) { *pos += 1; }
    }
    fn token(bytes: &[u8], pos: &mut usize) -> Value {
        let mut stream = serde_json::Deserializer::from_slice(&bytes[*pos..]).into_iter::<Value>();
        let value = stream.next().unwrap().unwrap();
        *pos += stream.byte_offset();
        value
    }
    fn skip(bytes: &[u8], pos: &mut usize) {
        let mut depth = 0usize;
        loop {
            match bytes.get(*pos) {
                Some(b'"') => {
                    *pos += 1;
                    while bytes[*pos] != b'"' {
                        if bytes[*pos] == b'\\' { *pos += 1; }
                        *pos += 1;
                    }
                    *pos += 1;
                    if depth == 0 { return; }
                }
                Some(b'{' | b'[') => { depth += 1; *pos += 1; }
                Some(b'}' | b']') if depth > 0 => {
                    depth -= 1; *pos += 1;
                    if depth == 0 { return; }
                }
                Some(c) if depth == 0 && (c.is_ascii_whitespace() || matches!(c, b',' | b'}' | b']')) => return,
                None => return,
                _ => { *pos += 1; }
            }
        }
    }
    fn walk(bytes: &[u8], pos: &mut usize, path: &str, wanted: &mut BTreeMap<String, usize>, newlines: &[usize]) {
        ws(bytes, pos);
        if !wanted.range(path.to_owned()..).next().is_some_and(|(p, _)| p.starts_with(path)) {
            skip(bytes, pos); return;
        }
        if let Some(line) = wanted.get_mut(path) { *line = newlines.partition_point(|&offset| offset < *pos) + 1; }
        match bytes[*pos] {
            b'{' => {
                *pos += 1;
                loop {
                    ws(bytes, pos);
                    if bytes[*pos] == b'}' { *pos += 1; break; }
                    let key = token(bytes, pos).as_str().unwrap().to_owned();
                    ws(bytes, pos); *pos += 1;
                    // Structural field names use dots; arbitrary table keys use brackets.
                    let bracket = format!("{path}[{key:?}]");
                    let opaque = wanted.range(bracket.clone()..).next().is_some_and(|(p, _)| p.starts_with(&bracket));
                    let child = if opaque { bracket } else { format!("{path}.{key}") };
                    walk(bytes, pos, &child, wanted, newlines);
                    ws(bytes, pos);
                    if bytes[*pos] == b',' { *pos += 1; }
                }
            }
            b'[' => {
                *pos += 1; let mut index = 0;
                loop {
                    ws(bytes, pos);
                    if bytes[*pos] == b']' { *pos += 1; break; }
                    walk(bytes, pos, &format!("{path}[{index}]"), wanted, newlines);
                    index += 1; ws(bytes, pos);
                    if bytes[*pos] == b',' { *pos += 1; }
                }
            }
            _ => { token(bytes, pos); }
        }
    }
    let mut wanted = origins.values().flatten().map(|path| (path.clone(), 0)).collect();
    let newlines = bytes.iter().enumerate().filter_map(|(i, &c)| (c == b'\n').then_some(i)).collect::<Vec<_>>();
    walk(bytes, &mut 0, "scene", &mut wanted, &newlines);
    for path in origins.values_mut().flatten() {
        if let Some(line) = wanted.get(path).filter(|&&line| line > 0) { *path = format!("{path} (line {line})"); }
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::super::Scene;

    #[test]
    fn malformed_resource_tables_are_errors() {
        for table in ["materials", "programs", "images", "buffers", "geometry"] {
            let mut document = serde_json::json!({"version":2,"nodes":[{"type":"camera"}]});
            document[table] = serde_json::json!([{}]);
            assert!(Scene::from_json(&serde_json::to_vec(&document).unwrap()).is_err(), "{table}");
        }
    }

    #[test]
    fn provenance_handles_numeric_segments_and_control_character_keys() {
        assert!(Scene::from_json(br#"{"version":2,"materials":[{}]}"#).is_err());
        let bytes = br#"{"version":2,"materials":{"\u0000":{"program":{"source":"broken shader"}}}}"#;
        let mut document = serde_json::from_slice(bytes).unwrap();
        let origins = super::lower(&mut document);
        assert_eq!(origins["materials.\0"], vec!["scene.materials[\"\\0\"]"]);
        assert!(origins.values().flatten().any(|path| path.ends_with(".program")));
        // Exercise source-location lookup as well as lowering.
        let _ = Scene::from_json_with_source(bytes, bytes, Some(std::path::Path::new("control.json")));
    }

    #[test]
    fn shared_inline_errors_keep_both_authored_slots_and_lines() {
        let bytes = br#"{
  "version": 2,
  "nodes": [
    {"type":"camera"},
    {"type":"mesh","geometry":"tri","materials":[{"inputs":{"subsurface_radius_scale":1}}]},
    {"type":"mesh","geometry":"tri","materials":[{"inputs":{"subsurface_radius_scale":1}}]}
  ],
  "geometry":{"tri":{"attributes":{"POSITION":{"type":"vec3","data":[[0,0,0],[1,0,0],[0,1,0]]}}}}
}"#;
        let error = Scene::from_json_with_source(bytes, bytes, Some(std::path::Path::new("caption.json"))).unwrap_err().to_string();
        assert!(error.contains("caption.json: scene.nodes[1].materials[0] (line 5)"), "{error}");
        assert!(error.contains("scene.nodes[2].materials[0] (line 6)"), "{error}");
        assert!(error.contains("std/openpbr"), "{error}");
        assert!(!error.contains('~'), "{error}");
    }

    #[test]
    fn shared_program_retains_each_owner_after_expansion_and_serialization() {
        let authored = br#"{
 "version":2,"nodes":[{"type":"camera"}],
 "materials": {
   "a[b": {"program":{"source":"broken shader"}},
   "second": {"program":{"source":"broken shader"}}
 }
}"#;
        let expanded = serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(authored).unwrap()).unwrap();
        let scene = Scene::from_json_with_source(&expanded, authored, Some(std::path::Path::new("shot.json"))).unwrap();
        assert_eq!(scene.programs.len(), 1);
        let key = format!("programs.{}", scene.programs.keys().next().unwrap());
        let users = &scene.origins[&key];
        assert_eq!(users.len(), 2, "{users:?}");
        assert!(users[0].contains("a[b\"].program (line 4)"), "{users:?}");
        assert!(users[1].contains("second\"].program (line 5)"), "{users:?}");
        assert!(scene.content_json().unwrap().get("origins").is_none());
    }
}
