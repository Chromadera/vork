//! Resolve named material attribute slots at installation; GPU readers use packed slots.
use anyhow::{Result, ensure};
use vork_scene::v2::{self, Input};

pub(crate) fn names(scene: &v2::Scene) -> Result<Vec<String>> {
    let names: std::collections::BTreeSet<_> = scene
        .materials
        .values()
        .flat_map(|m| m.inputs.values())
        .filter_map(|v| match v {
            Input::Attribute(b) => Some(b.attribute.clone()),
            _ => None,
        })
        .collect();
    Ok(names.into_iter().collect())
}

pub(crate) fn arity(ty: v2::BufferType) -> Result<u32> {
    Ok(match ty {
        v2::BufferType::F32 => 1,
        v2::BufferType::Vec2 => 2,
        v2::BufferType::Vec3 => 3,
        v2::BufferType::Vec4 => 4,
        _ => anyhow::bail!("attributes require float through float4"),
    })
}
// 4 means the destination-width prefix. Scalar destinations retain the r default;
// colour destinations retain rgb. Explicit scalar selectors broadcast.
pub(crate) fn channel(binding: &v2::AttributeBinding, width: u32) -> Result<u32> {
    use v2::Channel;
    Ok(match binding.channel {
        Some(Channel::R) => 0,
        Some(Channel::G) => 1,
        Some(Channel::B) => 2,
        Some(Channel::A) => 3,
        Some(Channel::Rgb) => {
            ensure!(width == 3, "rgb requires a float3 destination");
            4
        }
        None if width == 1 => 0,
        None => 4,
    })
}
pub(crate) fn factor(binding: &v2::AttributeBinding, width: u32) -> Result<[f32; 4]> {
    let mut result = [1.; 4];
    match &binding.factor {
        None => {}
        Some(v2::Factor::Number(v)) => result.fill(*v),
        Some(v2::Factor::Vector(v)) => {
            ensure!(
                v.len() == width as usize,
                "attribute factor requires {width} components"
            );
            result[..v.len()].copy_from_slice(v);
        }
    }
    ensure!(
        result.iter().all(|v| v.is_finite()),
        "attribute factor must be finite"
    );
    Ok(result)
}
pub(crate) fn validate(binding: &v2::AttributeBinding, width: u32, source: u32) -> Result<()> {
    let source = if source == 3 && v2::is_color_attribute(&binding.attribute) { 4 } else { source };
    let channel = channel(binding, width)?;
    if channel < 4 {
        ensure!(
            channel < source,
            "attribute {:?}: channel exceeds its {source} components",
            binding.attribute
        );
    } else {
        // float3 retains the existing rgb default for glTF COLOR float4.
        ensure!(
            source == width || (width == 3 && source == 4),
            "attribute {:?}: expected {width} components, found {source}",
            binding.attribute
        );
    }
    factor(binding, width)?;
    Ok(())
}

/// A binding has exactly one owner; ambiguity is an authoring error.
pub(crate) fn resolve<'a>(scene: &'a v2::Scene, node: &v2::Node, name: &str) -> Result<&'a str> {
    use anyhow::Context;
    let v2::NodeKind::Mesh { geometry, instances, .. } = &node.kind else { anyhow::bail!("attribute requires a native mesh") };
    // Return the scene-owned buffer key, rather than borrowing the node.
    let vertex = scene.geometry[geometry].attributes.get(name);
    let instance = instances.as_ref().and_then(|i| i.attributes.get(name));
    ensure!(vertex.is_none() || instance.is_none(), "attribute {name:?} exists on both geometry and instances");
    let buffer = vertex.or(instance).with_context(|| format!("missing attribute {name:?}"))?;
    if vertex.is_none() && scene.programs.values().any(|p| p.is_compute() && p.bindings.values().any(|b| b == buffer)) {
        eprintln!("warning: node {:?}: ignored compute updates to material attribute {name:?}; using authored values",node.name);
    }
    Ok(scene.buffers.get_key_value(buffer).context("unknown attribute buffer")?.0)
}

pub(crate) fn instance_value(
    node: &v2::Node, index: Option<u32>, name: &str, buffers: &crate::buffers::SceneBuffers,
) -> Option<[f32; 4]> {
    let v2::NodeKind::Mesh { instances: Some(instances), .. } = &node.kind else { return None };
    let buffer = &buffers.data[instances.attributes.get(name)?];
    let width = arity(buffer.element_type).ok()? as usize;
    let start = index? as usize * width;
    let mut value = [0.; 4];
    if width == 3 && v2::is_color_attribute(name) { value[3] = 1.; }
    for (out, word) in value.iter_mut().zip(&buffer.words[start..start + width]) { *out = f32::from_bits(*word); }
    Some(value)
}
