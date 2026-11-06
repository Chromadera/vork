//! Author-side standard geometry expansion. Output is an ordinary buffer scene.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub fn expand_scene(document: &mut Value, base: &Path) -> Result<()> {
    fn expand(value: &mut Value, base: &Path, built: &mut BTreeMap<String, Value>) -> Result<()> {
        let Some(shape) = value.get("std") else { return Ok(()) };
        let shape = shape.as_str().context("std must name a geometry builder")?.to_owned();
        let key = value.to_string();
        if let Some(result) = built.get(&key) { *value = result.clone(); return Ok(()) }
        let mut options = value.as_object().context("std geometry object")?.clone();
        options.remove("std");
        ensure!(!options.contains_key("name"), "std geometry uses its table key or inline position, not name");
        options.insert("name".into(), "shape".into());
        if shape == "text" {
            if let Some(font) = options.get("font").and_then(Value::as_str) {
                options.insert("font".into(), base.join(font).to_string_lossy().into_owned().into());
            }
        }
        let fragment: Value = serde_json::from_str(&super::evaluate(&shape, &Value::Object(options).to_string())?)?;
        let mut geometry = fragment.pointer("/geometry/shape").context("std builder must return geometry")?.clone();
        for attribute in geometry["attributes"].as_object_mut().context("std geometry attributes")?.values_mut() {
            let name = attribute.as_str().context("std attribute buffer name")?;
            *attribute = fragment["buffers"][name].clone();
        }
        if let Some(indices) = geometry.get_mut("indices") {
            let name = indices.as_str().context("std index buffer name")?;
            *indices = fragment["buffers"][name].clone();
        }
        built.insert(key, geometry.clone());
        *value = geometry;
        Ok(())
    }
    fn nodes(value: &mut Value, base: &Path, built: &mut BTreeMap<String, Value>) -> Result<()> {
        for node in value.as_array_mut().into_iter().flatten() {
            if let Some(geometry) = node.get_mut("geometry") {
                expand(geometry, base, built).context("inline node geometry")?;
            }
            if let Some(children) = node.get_mut("children") { nodes(children, base, built)?; }
        }
        Ok(())
    }
    let mut built = BTreeMap::new();
    if let Some(table) = document.get_mut("geometry").and_then(Value::as_object_mut) {
        for (name, geometry) in table {
            expand(geometry, base, &mut built).with_context(|| format!("geometry {name}"))?;
        }
    }
    if let Some(value) = document.get_mut("nodes") { nodes(value, base, &mut built)?; }
    Ok(())
}

pub fn load_scene(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut document = serde_json::from_slice(bytes.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(&bytes))?;
    expand_scene(&mut document, path.parent().unwrap_or(Path::new(".")))?;
    Ok(serde_json::to_vec(&document)?)
}
