//! Author-side standard geometry expansion. Output is an ordinary buffer scene.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub fn expand_scene(document: &mut Value, base: &Path) -> Result<()> {
    fn expand(value: &mut Value, base: &Path, built: &mut BTreeMap<String, Value>, apply: bool) -> Result<()> {
        let Some(shape) = value.get("std") else {
            return Ok(());
        };
        let shape = shape
            .as_str()
            .context("std must name a geometry builder")?
            .to_owned();
        let key = value.to_string();
        if let Some(result) = built.get(&key) {
            if apply { *value = result.clone(); }
            return Ok(());
        }
        let mut options = value.as_object().context("std geometry object")?.clone();
        options.remove("std");
        ensure!(
            !options.contains_key("name"),
            "std geometry uses its table key or inline position, not name"
        );
        options.insert("name".into(), "shape".into());
        if shape == "text" {
            if let Some(font) = options.get("font").and_then(Value::as_str) {
                options.insert(
                    "font".into(),
                    base.join(font).to_string_lossy().into_owned().into(),
                );
            }
        }
        let fragment: Value = serde_json::from_str(&super::evaluate(
            &shape,
            &Value::Object(options).to_string(),
        )?)?;
        let mut geometry = fragment
            .pointer("/geometry/shape")
            .context("std builder must return geometry")?
            .clone();
        for attribute in geometry["attributes"]
            .as_object_mut()
            .context("std geometry attributes")?
            .values_mut()
        {
            let name = attribute.as_str().context("std attribute buffer name")?;
            *attribute = fragment["buffers"][name].clone();
        }
        if let Some(indices) = geometry.get_mut("indices") {
            let name = indices.as_str().context("std index buffer name")?;
            *indices = fragment["buffers"][name].clone();
        }
        if apply { *value = geometry.clone(); }
        built.insert(key, geometry);
        Ok(())
    }
    fn nodes(value: &mut Value, base: &Path, built: &mut BTreeMap<String, Value>, errors: &mut Vec<anyhow::Error>, apply: bool) {
        for node in value.as_array_mut().into_iter().flatten() {
            if let Some(geometry) = node.get_mut("geometry") {
                if let Err(error) = expand(geometry, base, built, apply).context("inline node geometry") {
                    errors.push(error);
                }
            }
            if let Some(children) = node.get_mut("children") {
                nodes(children, base, built, errors, apply);
            }
        }
    }
    let mut built = BTreeMap::new();
    let mut errors = Vec::new();
    if let Some(table) = document.get_mut("geometry").and_then(Value::as_object_mut) {
        for (name, geometry) in table {
            if let Err(error) = expand(geometry, base, &mut built, false).with_context(|| format!("geometry {name}")) {
                errors.push(error);
            }
        }
    }
    if let Some(value) = document.get_mut("nodes") {
        nodes(value, base, &mut built, &mut errors, false);
    }
    if !errors.is_empty() {
        let messages = errors.iter().map(|error| format!("{error:#}")).collect::<Vec<_>>().join("\n");
        return Err(errors.remove(0).context(format!("{messages}\nCorrect each geometry builder or restore its font (choose a font covering the text); see vork help std --all")));
    }
    // Publish only after every builder succeeds. The existing deduplication map
    // owns the prepared geometry, so this walk does not evaluate builders again.
    if let Some(table) = document.get_mut("geometry").and_then(Value::as_object_mut) {
        for geometry in table.values_mut() { expand(geometry, base, &mut built, true)?; }
    }
    if let Some(value) = document.get_mut("nodes") {
        nodes(value, base, &mut built, &mut errors, true);
    }
    debug_assert!(errors.is_empty());
    Ok(())
}

pub fn load_scene(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut document =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
    expand_scene(&mut document, path.parent().unwrap_or(Path::new(".")))?;
    Ok(serde_json::to_vec(&document)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expansion_reports_independent_builder_failures_together() {
        let mut scene = json!({"version":2,"geometry":{
            "first":{"std":"not_a_builder"},"second":{"std":"also_not_a_builder"},"valid":{"std":"box"}},
            "nodes":[{"geometry":{"std":"third_invalid_builder"}},
                {"children":[{"geometry":{"std":"fourth_invalid_builder"}}]}]});
        let original = scene.clone();
        let error = format!("{:#}", expand_scene(&mut scene, Path::new(".")).unwrap_err());
        assert_eq!(scene, original, "a failed expansion must not publish successful siblings");
        for name in ["not_a_builder", "also_not_a_builder", "third_invalid_builder", "fourth_invalid_builder"] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn expansion_owns_identity_and_preserves_unvalidated_content() -> Result<()> {
        let definition = json!({"std":"rounded_box","radius":0});
        let mut document = json!({"version":"invalid", "programs":{"broken":{"unknown":true}},
            "geometry":{"custom":definition},
            "nodes":[{"children":[{"geometry":definition}]}]});
        let expanded: Value =
            serde_json::from_str(&super::super::evaluate("expand", &document.to_string())?)?;
        let table = &expanded["geometry"]["custom"];
        assert_eq!(table, &expanded["nodes"][0]["children"][0]["geometry"]);
        assert!(table.get("std").is_none());
        assert!(table["attributes"]["POSITION"]["data"].is_array());
        assert!(table["indices"]["data"].is_array());
        assert_eq!(expanded["version"], document["version"]);
        assert_eq!(expanded["programs"], document["programs"]);
        for pointer in ["/geometry/custom", "/nodes/0/children/0/geometry"] {
            document.pointer_mut(pointer).unwrap()["name"] = "forbidden".into();
            let error = format!(
                "{:#}",
                expand_scene(&mut document, Path::new(".")).unwrap_err()
            );
            assert!(error.contains("table key or inline position"), "{error}");
            document
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("name");
        }
        Ok(())
    }
}
