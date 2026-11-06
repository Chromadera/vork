//! Shared conservative mesh-source support in the existing ray table.
use crate::*;

// The proposal domain consists of actual AS instances, not merely installed
// draws. Both the allocation gate and table builder use the same support fact.
fn supports(draw: &install::Draw, material: &Material, physical: bool) -> bool {
    // Implicit sources use a separate direction proposal over their bounds;
    // bounds are not triangle-area sources.
    // Invisible emission must not reserve source connections: even a rejected
    // connection changes the reflection strategy and its finite-sample result.
    draw.implicit.is_none() && draw.count>0 && !material.zero_opacity
        && material.source_emission && (!physical || !material.transparent)
}
pub(crate) fn has_sources(draws: &[install::Draw], instances: &[(usize,u32)], materials: &[Material], physical: bool) -> bool {
    instances.iter().any(|&(draw,_)| supports(&draws[draw],&materials[draws[draw].material],physical))
}

// Immutable membership, not a second copy of geometry or material values. The
// frame-private emitter_weights tree supplies current world-area probabilities;
// the same current corners supply the shader solid-angle Jacobian.
// Header: source count, total triangles, instance count; then instance -> source (or MAX), then
// sources [instance, object, cumulative triangles]. Binary search is O(log N).
pub(crate) fn emitter_topology(
    draws: &[install::Draw], instances: &[(usize, u32)], materials: &[Material], physical: bool,
) -> Result<Vec<u32>> {
    let mut words = vec![0, 0, instances.len() as u32];
    words.resize(3 + instances.len(), u32::MAX);
    for (instance, &(draw, object)) in instances.iter().enumerate() {
        let draw = &draws[draw];
        if !supports(draw,&materials[draw.material],physical) {continue;}
        words[1] = words[1].checked_add(draw.count / 3).context("too many emitter triangles")?;
        words[3 + instance] = words[0];
        words[0] += 1;
        words.extend([instance as u32, object, words[1]]);
    }
    Ok(words)
}


// Same immutable membership policy as physical triangle emitters, but without
// pretending the implicit draw's installation box is an emitting surface.
pub(crate) fn implicit_sources(draws:&[install::Draw],instances:&[(usize,u32)],materials:&[Material])->Vec<u32> {
    instances.iter().filter(|&&(draw,_)|draws[draw].implicit.is_some()).enumerate()
        .filter_map(|(field,&(draw,_))| {
            let material=&materials[draws[draw].material];
            (material.source_emission && !material.transparent && !material.zero_opacity).then_some(field as u32)
        }).collect()
}
