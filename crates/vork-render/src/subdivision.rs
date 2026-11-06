//! Install-time subdivision stencils and CPU evaluation after live deformation.
//! Static meshes finish here. Animated output uses the existing two-pose morph
//! stream, dirty uploads and deformation dispatch; no new pass or shader.
use crate::*;
use ::vork_scene::v2;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use vork_mesh::Plan;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Refinement {
    pub(crate) plan: Plan,
    pub(crate) positions: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
}
impl Refinement {
    pub(crate) fn evaluate(&self, cage: &[[f32; 3]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let positions = self.plan.evaluate(cage);
        let normals = self.plan.normals(&positions);
        (positions, normals)
    }
}
#[derive(Serialize, Deserialize)]
struct Baked {
    refinement: Refinement,
    geometry: crate::geometry::Geometry,
}

pub(crate) fn prepare(
    r: &Renderer,
    buffers: &buffers::SceneBuffers,
    source: &v2::Geometry,
    geometry: crate::geometry::Geometry,
) -> Result<crate::geometry::Geometry> {
    if source
        .subdivision
        .as_ref()
        .is_none_or(|s| s.render_levels == 0)
    {
        return Ok(geometry);
    }
    ensure!(
        source.material_ranges.is_empty(),
        "subdivision requires a single-material cage (no material_ranges)"
    );
    let mut key = blake3::Hasher::new();
    key.update(&serde_json::to_vec(source)?);
    for name in source
        .attributes
        .values()
        .chain(source.indices.iter())
        .chain(
            source
                .morphs
                .values()
                .flat_map(|m| std::iter::once(&m.POSITION).chain(m.NORMAL.iter())),
        )
    {
        for word in &buffers.data[name].words {
            key.update(&word.to_le_bytes());
        }
    }
    let indices = source
        .indices
        .as_ref()
        .map(|n| buffers.data[n].words.clone())
        .unwrap_or_else(|| (0..geometry.vertices.len() as u32).collect());
    let (levels, simple, limit) = source.subdivision.as_ref().map_or((0, false, false), |s| {
        (
            s.render_levels,
            matches!(s.subdivision_type, v2::SubdivisionType::Simple),
            s.use_limit_surface,
        )
    });
    let (vertices, indices_count) = Plan::refined_counts(geometry.vertices.len(), &indices, source.face_vertex_counts.as_deref(), levels)?;
    let limits = r.device.limits();
    // Every output vertex has a float3 position. Other retained streams check
    // their exact byte demands when packing; do not impose a full-Vertex stride
    // on a split position/attribute buffer.
    crate::gpu_memory::storage_binding(&limits, "subdivision positions", vertices * 12)?;
    ensure!(indices_count*4 <= limits.max_buffer_size,
        "subdivision triangle indices need {} bytes; device max_buffer_size={} bytes", indices_count*4, limits.max_buffer_size);
    let path = r.disk_cache.path("subdivision", 1, key.finalize());
    if let Some(bytes) = disk_cache::read(&path) {
        if let Ok(mut baked) = serde_json::from_slice::<Baked>(&bytes) {
            baked.geometry.refinement = Some(Arc::new(baked.refinement));
            return Ok(baked.geometry);
        }
    }
    let plan = Plan::new(
        geometry.vertices.len(),
        &indices,
        source.face_vertex_counts.as_deref(),
        &source.crease_edge.iter().map(|e| (e.vertices, e.crease)).collect::<Vec<_>>(),
        levels,
        simple,
        limit,
    )?;
    let positions: Vec<_> = geometry.vertices.iter().map(|v| v.POSITION).collect();
    let uv = plan.evaluate(
        &geometry
            .vertices
            .iter()
            .map(|v| v.TEXCOORD_0)
            .collect::<Vec<_>>(),
    );
    let refinement = Refinement {
        plan,
        positions,
        uv,
    };
    let (positions, normals) = refinement.evaluate(&refinement.positions);
    ensure!(
        positions.iter().flatten().all(|v| v.is_finite()),
        "subdivision produced non-finite positions"
    );
    let uv1 = refinement.plan.evaluate(
        &geometry
            .vertices
            .iter()
            .map(|v| v.TEXCOORD_1)
            .collect::<Vec<_>>(),
    );
    let colors = refinement.plan.evaluate(
        &geometry
            .vertices
            .iter()
            .map(|v| v.COLOR_0)
            .collect::<Vec<_>>(),
    );
    let uvs = refinement
        .uv
        .iter()
        .zip(uv1)
        .map(|(a, b)| [*a, b])
        .collect();
    let mut result = crate::geometry::Geometry::channels(
        positions,
        normals,
        uvs,
        refinement.plan.indices.clone(),
        geometry.uv_sets,
    );
    result.primitive_ids=if levels==0 {geometry.primitive_ids.clone()} else {refinement.plan.primitive_ids.clone()};
    for (v, color) in result.vertices.iter_mut().zip(colors) {
        v.COLOR_0 = color;
    }
    for (name, values) in &geometry.attributes {
        result
            .attributes
            .insert(name.clone(), refinement.plan.evaluate(values));
    }
    // Reconstruct tangent space for the new topology and normals.
    tangents(&mut result);
    let baked = Baked {
        refinement: refinement.clone(),
        geometry: result,
    };
    if let Ok(bytes) = serde_json::to_vec(&baked) {
        let _ = disk_cache::write(&path, &bytes);
    }
    let mut result = baked.geometry;
    result.refinement = Some(Arc::new(refinement));
    Ok(result)
}
fn tangents(geometry: &mut crate::geometry::Geometry) {
    let mut t = vec![Vec3::ZERO; geometry.vertices.len()];
    let mut b = t.clone();
    for face in geometry.indices.chunks_exact(3) {
        let a = &geometry.vertices[face[0] as usize];
        let x = &geometry.vertices[face[1] as usize];
        let y = &geometry.vertices[face[2] as usize];
        let e1 = Vec3::from(x.POSITION) - Vec3::from(a.POSITION);
        let e2 = Vec3::from(y.POSITION) - Vec3::from(a.POSITION);
        let u1 = glam::Vec2::from(x.TEXCOORD_0) - glam::Vec2::from(a.TEXCOORD_0);
        let u2 = glam::Vec2::from(y.TEXCOORD_0) - glam::Vec2::from(a.TEXCOORD_0);
        let det = u1.perp_dot(u2);
        if det.abs() > f32::EPSILON {
            for &i in face {
                t[i as usize] += (e1 * u2.y - e2 * u1.y) / det;
                b[i as usize] += (e2 * u1.x - e1 * u2.x) / det;
            }
        }
    }
    for (i, v) in geometry.vertices.iter_mut().enumerate() {
        let n = Vec3::from(v.NORMAL);
        let tangent = (t[i] - n * n.dot(t[i])).normalize_or_zero();
        v.TANGENT = tangent
            .extend(if n.cross(tangent).dot(b[i]) < 0. {
                -1.
            } else {
                1.
            })
            .to_array();
    }
}

pub(crate) struct CpuDeform {
    pub(crate) base: Vec<Vertex>,
    refinement: Arc<Refinement>,
    influences: Vec<Influence>,
    morphs: Vec<[f32; 3]>,
}
impl CpuDeform {
    pub(crate) fn new(
        refinement: Arc<Refinement>,
        mut influences: Vec<Influence>,
        morphs: Vec<[f32; 3]>,
        joints: usize,
    ) -> Result<Self> {
        for influence in &mut influences {
            let sum: f32 = influence.WEIGHTS_0.iter().sum();
            ensure!(
                sum.is_finite()
                    && sum > 0.
                    && influence.WEIGHTS_0.iter().all(|w| w.is_finite() && *w >= 0.)
                    && influence.JOINTS_0.iter().all(|j| (*j as usize) < joints),
                "invalid subdivision skin weights"
            );
            for weight in &mut influence.WEIGHTS_0 {
                *weight /= sum;
            }
        }
        Ok(Self {
            base: Vec::new(),
            refinement,
            influences,
            morphs,
        })
    }
    pub(crate) fn vertices(&self, matrices: &[Mat4], weights: &[f32]) -> Vec<Vertex> {
        let (positions, normals) = self.evaluate(matrices, weights);
        let mut geometry = crate::geometry::Geometry::new(self.base.clone(), self.refinement.plan.indices.clone(), 1);
        for ((v,p),n) in geometry.vertices.iter_mut().zip(positions).zip(normals) {v.POSITION=p;v.NORMAL=n;}
        tangents(&mut geometry);
        geometry.vertices
    }
    pub(crate) fn evaluate(
        &self,
        matrices: &[Mat4],
        weights: &[f32],
    ) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let count = self.refinement.positions.len();
        let cage: Vec<_> = self
            .refinement
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut p = Vec3::from(*p);
                for (target, &weight) in weights.iter().enumerate() {
                    if weight != 0. {
                        p += Vec3::from(self.morphs[target * count + i]) * weight;
                    }
                }
                let inf = &self.influences[i];
                let mut out = Vec3::ZERO;
                for (&joint, &weight) in inf.JOINTS_0.iter().zip(&inf.WEIGHTS_0) {
                    if weight != 0. {
                        out += matrices[joint as usize].transform_point3(p) * weight;
                    }
                }
                out.to_array()
            })
            .collect();
        self.refinement.evaluate(&cage)
    }
}

/// Each frame slot owns its two-pose CPU results. Alias draws share the leader.
pub(crate) fn frame_resources(
    s: &InstalledScene,
    r: &Renderer,
    frames: &mut [Option<FrameDeform>],
) -> Vec<Option<wgpu::Buffer>> {
    let mut buffers: Vec<Option<wgpu::Buffer>> = Vec::with_capacity(s.draws.len());
    for (i, draw) in s.draws.iter().enumerate() {
        let Some(deform) = draw.deform.as_ref().filter(|d| d.cpu.is_some()) else {
            buffers.push(None);
            continue;
        };
        if deform.leader < i {
            buffers.push(buffers[deform.leader].clone());
            continue;
        }
        let morphs = crate::gpu_memory::buffer(&r.device, &wgpu::BufferDescriptor {
            label: Some("frame subdivision poses"),
            size: deform.count as u64 * 3 * constants::MORPH_FLOATS as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame = frames[i].as_mut().unwrap();
        let pipeline = s.pipelines.deform_pipeline.get().unwrap();
        frame.bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CPU subdivision through deformation"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, deform.source.as_entire_binding()),
                entry(1, deform.influences.as_entire_binding()),
                entry(2, morphs.as_entire_binding()),
                entry(3, frame.params.as_entire_binding()),
                entry(4, frame.output.binding()),
                entry(5, frame.previous_params.as_entire_binding()),
                entry(6, frame.previous_output.as_entire_binding()),
            ],
        });
        buffers.push(Some(morphs));
    }
    buffers
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subdivision_follows_skinning_and_morphs() -> Result<()> {
        let p = vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
        let refinement = Arc::new(Refinement {
            plan: Plan::new(4, &[0, 1, 2, 3], Some(&[4]), &[], 1, true, false)?,
            positions: p,
            uv: vec![[0.; 2]; 9],
        });
        let influences = (0..4)
            .map(|i| Influence {
                JOINTS_0: [if i == 1 || i == 2 { 1 } else { 0 }; 4],
                WEIGHTS_0: [1., 0., 0., 0.],
            })
            .collect();
        let cpu = CpuDeform::new(
            refinement,
            influences,
            vec![[0., 0., 0.], [0., 0., 0.], [0., 0.4, 0.], [0., 0., 0.]],
            2,
        )?;
        let (positions, _) = cpu.evaluate(
            &[
                Mat4::IDENTITY,
                Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ],
            &[1.],
        );
        assert!(
            positions
                .iter()
                .any(|p| Vec3::from(*p).abs_diff_eq(Vec3::new(0., 0.6, -0.5), 1e-6)),
            "subdivide the deformed cage, rather than skinning interpolated bind vertices"
        );
        Ok(())
    }
}
