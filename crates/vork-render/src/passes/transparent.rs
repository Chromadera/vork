//! Nearest-surface controls of transparent draws (colour is the decoupled shade's).
use crate::*;

/// A total float order keeps the previous comparator's signed-zero/NaN semantics.
#[derive(Clone, Copy)]
struct Depth(f32);
impl PartialEq for Depth {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Depth {}
impl PartialOrd for Depth {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Depth {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Back-to-front draw order breaks equal-depth ties in the transparent controls pass.
/// Consecutive geometry/material runs share instanced controls draws.
pub(crate) fn sort(
    s: &InstalledScene,
    objects: &[Object],
    order: &mut [usize],
    view: Mat4,
    instancing: bool,
) -> Option<(
    Vec<(usize, u32)>,
    Vec<wgpu::util::DrawIndexedIndirectArgs>,
    Vec<usize>,
)> {
    order.sort_by_cached_key(|&i| {
        let center = objects[s.draws[i].object as usize]
            .model
            .transform_point3(s.draws[i].center);
        (
            Depth(view.transform_point3(center).z),
            s.draws[i].sort_order,
        )
    });
    if !instancing {
        return None;
    }
    let mut values = Vec::with_capacity(order.len());
    let mut args = vec![wgpu::util::DrawIndexedIndirectArgs::default(); s.draws.len()];
    let mut next = vec![0; s.draws.len()];
    let mut runs: Vec<usize> = Vec::new();
    let mut last_group = None;
    for &id in order.iter() {
        let group = &s.draw_groups[id];
        let slot = group.start + next[group.start];
        next[group.start] += 1;
        values.push((
            s.draws[slot].object as usize,
            s.draws[id].object + 1,
        ));
        if last_group == Some(group.start) {
            args[*runs.last().unwrap()].instance_count += 1;
        } else {
            args[slot] = wgpu::util::DrawIndexedIndirectArgs {
                index_count: s.draws[id].count,
                instance_count: 1,
                first_index: 0,
                base_vertex: 0,
                first_instance: s.draws[slot].object,
            };
            runs.push(slot);
            last_group = Some(group.start);
        }
    }
    Some((values, args, runs))
}
fn draw_order(f: &crate::view::ViewState) -> &[usize] {
    if f.transparent_indirect.is_some() {
        &f.transparent_runs
    } else {
        &f.transparent_order
    }
}
pub(crate) fn encode_controls(encoder: &mut wgpu::CommandEncoder, f: &crate::view::ViewState) {
    if f.transparent_order.is_empty() {
        return;
    }
    let mut pass = super::forward::begin_pass(encoder,f,"nearest transparent controls",wgpu::LoadOp::Load,Some("transparency"));
    let bundles = f.bundles();
    pass.execute_bundles(
        draw_order(f)
            .iter()
            .map(|&i| bundles.transparent[i].as_ref().unwrap()),
    );
}
