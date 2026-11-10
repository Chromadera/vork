//! Install-time point spheres; the resulting mesh follows the ordinary retained path.
use super::*;

pub(crate) fn triangles_per_point(sides: usize) -> Result<u64> {
    (sides as u64).checked_mul(2)
        .and_then(|n| n.checked_mul(sides.div_ceil(2).saturating_sub(1) as u64))
        .context("point triangle count overflow")
}

// Persist only generated channels. Constant colour/tangent/padding are reconstructed.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
#[allow(non_snake_case)]
struct CachedVertex {
    POSITION: [f32; 3],
    NORMAL: [f32; 3],
    TEXCOORD_0: [f32; 2],
}

pub(crate) fn cached_spheres(
    positions: &[[f32; 3]],
    widths: &[f32],
    sides: usize,
    cache: Option<&crate::disk_cache::Store>,
) -> Result<crate::geometry::Geometry> {
    let mut hash = blake3::Hasher::new();
    hash.update(bytemuck::cast_slice(positions));
    hash.update(bytemuck::cast_slice(widths));
    hash.update(&(positions.len() as u64).to_le_bytes());
    hash.update(&(widths.len() as u64).to_le_bytes());
    hash.update(&(sides as u64).to_le_bytes());
    hash.update(&(sides.div_ceil(2) as u64).to_le_bytes());
    let path = cache.map(|c| c.path("point-spheres", 1, hash.finalize()));
    if let Some(payload) = path.as_ref().and_then(|p| crate::disk_cache::read(p)) {
        if let Some(mesh) = decode(&payload) {
            return Ok(mesh);
        }
    }
    let mesh = spheres(positions, widths, sides)?;
    if let Some(path) = path {
        let header = [mesh.vertices.len() as u64, mesh.indices.len() as u64];
        let vertices: Vec<_> = mesh
            .vertices
            .iter()
            .map(|v| CachedVertex {
                POSITION: v.POSITION,
                NORMAL: v.NORMAL,
                TEXCOORD_0: v.TEXCOORD_0,
            })
            .collect();
        let _ = crate::disk_cache::write_parts(
            &path,
            &[
                bytemuck::cast_slice(&header),
                bytemuck::cast_slice(&vertices),
                bytemuck::cast_slice(&mesh.indices),
            ],
        );
    }
    Ok(mesh)
}

fn decode(payload: &[u8]) -> Option<crate::geometry::Geometry> {
    let header = payload.get(..16)?;
    let vertices = usize::try_from(u64::from_ne_bytes(header[..8].try_into().ok()?)).ok()?;
    let indices = usize::try_from(u64::from_ne_bytes(header[8..].try_into().ok()?)).ok()?;
    let vertex_end =
        16usize.checked_add(vertices.checked_mul(std::mem::size_of::<CachedVertex>())?)?;
    let end = vertex_end.checked_add(indices.checked_mul(4)?)?;
    if end != payload.len() || indices % 3 != 0 {
        return None;
    }
    let vertices: Vec<Vertex> = payload[16..vertex_end]
        .chunks_exact(std::mem::size_of::<CachedVertex>())
        .map(|bytes| {
            let v: CachedVertex = bytemuck::pod_read_unaligned(bytes);
            Vertex {
                POSITION: v.POSITION,
                NORMAL: v.NORMAL,
                TEXCOORD_0: v.TEXCOORD_0,
                TEXCOORD_1: [0.; 2],
                pad: [0.; 2],
                COLOR_0: [1.; 4],
                TANGENT: [0.; 4],
            }
        })
        .collect();
    let indices: Vec<u32> = payload[vertex_end..]
        .chunks_exact(4)
        .map(bytemuck::pod_read_unaligned)
        .collect();
    if indices.iter().any(|&i| i as usize >= vertices.len()) {
        return None;
    }
    Some(crate::geometry::Geometry::new(vertices, indices, 1))
}

pub(crate) fn spheres(positions: &[[f32; 3]], widths: &[f32], sides: usize) -> Result<crate::geometry::Geometry> {
    ensure!(
        !positions.is_empty(),
        "points require at least one position"
    );
    ensure!(
        positions.len() == widths.len(),
        "point widths count must match positions"
    );
    ensure!(
        positions.iter().flatten().all(|v| v.is_finite())
            && widths.iter().all(|w| w.is_finite() && *w >= 0.),
        "point positions must be finite and widths finite and nonnegative"
    );
    ensure!(sides >= 8, "point sphere requires at least 8 radial sides");
    let vertices_per_point = sides.checked_add(1).and_then(|n| n.checked_mul(sides.div_ceil(2) + 1)).context("point tessellation overflow")?;
    let count = positions
        .len()
        .checked_mul(vertices_per_point)
        .context("point vertex count overflow")?;
    ensure!(
        count <= u32::MAX as usize,
        "points exceed u32 vertex indices"
    );
    let shape = shape(sides)?;
    let mut vertices = Vec::with_capacity(count);
    let index_count = positions.len().checked_mul(shape.indices.len())
        .context("point index count overflow")?;
    let mut indices = Vec::with_capacity(index_count);
    for (&position, &width) in positions.iter().zip(widths) {
        let base = vertices.len() as u32;
        for vertex in &shape.vertices {
            let mut vertex = *vertex;
            let p = Vec3::from(position) + Vec3::from(vertex.NORMAL) * (width * 0.5);
            ensure!(p.is_finite(), "point sphere positions overflow");
            vertex.POSITION = p.to_array();
            vertices.push(vertex);
        }
        indices.extend(shape.indices.iter().map(|index| base + index));
    }
    Ok(crate::geometry::Geometry::new(vertices, indices, 1))
}

/// One immutable unit-radius sphere. Point placement never changes this topology,
/// including seam vertices, pole handling and primitive order.
pub(crate) fn shape(sides: usize) -> Result<crate::geometry::Geometry> {
    ensure!(sides >= 8, "point sphere requires at least 8 radial sides");
    let count = sides.checked_add(1)
        .and_then(|n| n.checked_mul(sides.div_ceil(2).checked_add(1)?))
        .context("point tessellation overflow")?;
    ensure!(count <= u32::MAX as usize, "point shape exceeds u32 vertex indices");
    let mut vertices = Vec::with_capacity(count);
    let mut indices = Vec::new();
    let longitude = sides;
    let latitude = sides.div_ceil(2);
    for y in 0..=latitude {
        let theta = std::f32::consts::PI * y as f32 / latitude as f32;
        for x in 0..=longitude {
            let phi = std::f32::consts::TAU * (x % longitude) as f32 / longitude as f32;
            let normal = if y == 0 {
                Vec3::Y
            } else if y == latitude {
                -Vec3::Y
            } else {
                Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
            };
            vertices.push(Vertex {
                POSITION: normal.to_array(), NORMAL: normal.to_array(),
                TEXCOORD_0: [x as f32 / longitude as f32, y as f32 / latitude as f32],
                TEXCOORD_1: [0.; 2], pad: [0.; 2], COLOR_0: [1.; 4], TANGENT: [0.; 4],
            });
        }
    }
    for y in 0..latitude {
        for x in 0..longitude {
            let a = (y * (longitude + 1) + x) as u32;
            let b = a + 1;
            let c = a + (longitude + 1) as u32;
            let d = c + 1;
            if y > 0 { indices.extend_from_slice(&[a, b, c]); }
            if y + 1 < latitude { indices.extend_from_slice(&[b, d, c]); }
        }
    }
    Ok(crate::geometry::Geometry::new(vertices, indices, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_shape_preserves_closed_topology_and_point_correspondence() -> Result<()> {
        for sides in [8, 9, 418] {
            let shape = shape(sides)?;
            assert_eq!(shape.indices.len() as u64, triangles_per_point(sides)? * 3);
            let positions = [[3., -2., 4.], [-1., 7., 2.], [0.; 3]];
            let widths = [0.5, 2., 0.];
            let expanded = spheres(&positions, &widths, sides)?;
            for point in 0..positions.len() {
                let first = point * shape.vertices.len();
                for (local, vertex) in shape.vertices.iter().enumerate() {
                    let actual = &expanded.vertices[first + local];
                    let expected = Vec3::from(positions[point]) + Vec3::from(vertex.NORMAL) * (widths[point] * 0.5);
                    assert_eq!(actual.POSITION, expected.to_array());
                    assert_eq!(actual.NORMAL, vertex.NORMAL);
                    assert_eq!(actual.TEXCOORD_0, vertex.TEXCOORD_0);
                }
                let first_index = point * shape.indices.len();
                for (local, index) in shape.indices.iter().enumerate() {
                    assert_eq!(expanded.indices[first_index + local], first as u32 + index);
                }
            }
            let (edges, degenerate) = crate::geometry::edge_uses(&shape.indices, |i| {
                crate::geometry::weld_key(shape.vertices[i as usize].POSITION)
            });
            assert!(!degenerate);
            assert!(edges.values().all(|&(uses, winding)| uses == 2 && winding == 0));
        }
        Ok(())
    }
}
