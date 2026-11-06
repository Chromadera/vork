//! Hair geometry packing and native wrappers; scattering lives in std/hair_bsdf.
use anyhow::{Result, ensure};

/// Retained exit trees, partitioned per strand. End disks only
/// bound the receiving fiber's interior; they do not change raster/RT geometry.
pub(crate) fn geometry_words(geometry: &crate::geometry::Geometry) -> Result<Vec<u32>> {
    let strands = geometry.vertices.last().map_or(0, |v| v.TEXCOORD_1[0] as usize + 1);
    let mut words = vec![0; strands * 4];
    let mut strand = 0;
    let mut vertex = 0;
    let mut index = 0;
    while vertex < geometry.vertices.len() {
        let first = &geometry.vertices[vertex];
        let fiber = first.TEXCOORD_1;
        let ring = geometry.vertices[vertex..].iter().take_while(|v| v.TEXCOORD_1 == fiber && v.TEXCOORD_0[1] == first.TEXCOORD_0[1]).count();
        ensure!(ring >= 4, "invalid curve ring");
        let sides = ring - 1;
        let caps = 2 * (sides - 2);
        let triangles = fiber[1] as usize;
        ensure!(
            fiber[0] as usize == strand && triangles >= caps,
            "invalid curve shadow range"
        );
        let side_triangles = triangles - caps;
        let segments = side_triangles / (2 * sides);
        let mut positions = Vec::with_capacity(triangles);
        let mut triangle = |a: usize, b: usize, c: usize| {
            positions.push([a,b,c].map(|i| geometry.vertices[i].POSITION));
        };
        for indices in geometry.indices[index..index + side_triangles * 3].chunks_exact(3) {
            triangle(
                indices[0] as usize,
                indices[1] as usize,
                indices[2] as usize,
            );
        }
        let end = vertex + segments * ring;
        for j in 1..sides - 1 {
            triangle(vertex, vertex + j + 1, vertex + j);
            triangle(end, end + j, end + j + 1);
        }
        let tree = crate::mesh_exit::tree(&positions);
        words[strand*4] = u32::try_from(words.len())?;
        words[strand*4+1] = u32::try_from(tree.nodes.len())?;
        words.extend_from_slice(bytemuck::cast_slice(&tree.nodes));
        words[strand*4+2] = u32::try_from(words.len())?;
        words[strand*4+3] = triangles as u32;
        let corners: Vec<[crate::mesh_exit::ExitVertex;3]> = tree.triangles.iter()
            .map(|&i| positions[i as usize].map(|value| crate::mesh_exit::ExitVertex {value, vertex:0})).collect();
        words.extend_from_slice(bytemuck::cast_slice(&corners));
        strand += 1;
        index += side_triangles * 3;
        vertex += (segments + 1) * ring;
    }
    ensure!(
        index == geometry.indices.len(),
        "curve shadow triangles do not partition geometry"
    );
    Ok(words)
}
