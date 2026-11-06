//! Retained stackless acceleration shared by solid and receiving-strand exits.
use crate::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ExitVertex {
    pub value: [f32; 3],
    pub vertex: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Node {
    lo: [f32; 3],
    // Internal escape index, or the bounded leaf triangle count.
    next: u32,
    hi: [f32; 3],
    // Leaf packet offset; u32::MAX marks an internal node.
    first: u32,
}

const _: () = assert!(std::mem::size_of::<Node>() == vork_slang::registry::EXIT_NODE_VEC4S * 16);

const _: () = assert!(
    std::mem::size_of::<[ExitVertex; 3]>() == vork_slang::registry::EXIT_TRIANGLE_VEC4S * 16
);

pub(crate) struct Tree {
    pub nodes: Vec<Node>,
    pub triangles: Vec<u32>,
}

/// Compact bounds and small triangle packets share one traversal for all exits.
pub(crate) fn tree(positions: &[[[f32; 3]; 3]]) -> Tree {
    fn build(positions: &[[[f32; 3]; 3]], triangles: &mut [u32], tree: &mut Tree) {
        let nodes = &mut tree.nodes;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &triangle in triangles.iter() {
            for &p in &positions[triangle as usize] {
                lo = lo.min(Vec3::from(p));
                hi = hi.max(Vec3::from(p));
            }
        }
        let root = nodes.len();
        nodes.push(Node {
            lo: lo.to_array(),
            hi: hi.to_array(),
            next: 0,
            first: u32::MAX,
        });
        if triangles.len() <= crate::constants::EXIT_LEAF_TRIANGLES {
            nodes[root].first = tree.triangles.len() as u32;
            nodes[root].next = triangles.len() as u32;
            tree.triangles.extend_from_slice(triangles);
        } else {
            let extent = hi - lo;
            let axis = (0..3).find(|&i| extent[i] == extent.max_element()).unwrap();
            let mid = triangles.len() / 2;
            triangles.select_nth_unstable_by(mid, |&a, &b| {
                let centroid = |t: u32| positions[t as usize].iter().map(|p| p[axis]).sum::<f32>();
                centroid(a).total_cmp(&centroid(b))
            });
            let (left, right) = triangles.split_at_mut(mid);
            build(positions, left, tree);
            build(positions, right, tree);
            tree.nodes[root].next = tree.nodes.len() as u32;
        }
    }
    let mut tree = Tree {
        nodes: Vec::new(),
        triangles: Vec::new(),
    };
    let mut triangles: Vec<_> = (0..positions.len() as u32).collect();
    if !triangles.is_empty() {
        build(positions, &mut triangles, &mut tree);
    }
    tree
}

#[cfg(test)]
#[path = "mesh_exit_tests.rs"]
mod tests;
