//! Mesh-local exit intersections. The retained tree shares raster geometry and transforms.
use crate::*;

use crate::mesh_exit::{tree, ExitVertex};

pub(crate) fn mesh(
    d: &wgpu::Device,
    draw: &install::Draw,
    geometry: &geometry::Geometry,
) -> wgpu::Buffer {
    let positions: Vec<_> = geometry.indices.chunks_exact(3)
        .map(|indices| std::array::from_fn(|j| geometry.vertices[indices[j] as usize].POSITION)).collect();
    let tree = tree(&positions);
    // Static exits retain their positions beside the normals, avoiding a second
    // indexed read through the raster stream for every triangle intersection.
    // Deformed exits must continue reading this frame's raster positions.
    let mut corners = Vec::<[ExitVertex;3]>::new();
    for &triangle in &tree.triangles {
        corners.push(std::array::from_fn(|j| {
            let vertex = geometry.indices[triangle as usize * 3 + j];
            ExitVertex { value: geometry.vertices[vertex as usize].NORMAL, vertex }
        }));
        if !draw.evaluated() {
            corners.push(positions[triangle as usize].map(|value| ExitVertex { value, vertex: 0 }));
        }
    }
    // Scalar header; current-pose layout is already in the shared hit record.
    let mut bytes = bytemuck::bytes_of(&(tree.nodes.len() as u32)).to_vec();
    bytes.extend_from_slice(bytemuck::cast_slice(&tree.nodes));
    bytes.extend_from_slice(bytemuck::cast_slice(&corners));
    init(d, "solid exit tree", &bytes, wgpu::BufferUsages::STORAGE)
}

#[cfg(all(test, vork_private_tests))]
include!("transmission_tests_private.rs");
