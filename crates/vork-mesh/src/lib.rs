//! Unstable, internal: all Rust APIs in this crate are free to change.
//!
//! CPU Catmull–Clark stencils. Topology is planned once; positions may be deformed
//! before evaluation, matching a Subdivision Surface after an Armature modifier.
use anyhow::{Result, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type Stencil = Vec<(u32, f32)>;
#[derive(Clone, Serialize, Deserialize)]
pub struct Plan {
    pub stencils: Vec<Stencil>,
    pub indices: Vec<u32>,
    /// Original control-face ordinal for each output triangle.
    pub primitive_ids: Vec<u32>,
}
#[derive(Default)]
struct Edge {
    a: usize,
    faces: Vec<usize>,
    sharp: f32,
}
fn topology(
    faces: &[Vec<u32>],
    count: usize,
    sharp: &BTreeMap<(u32, u32), f32>,
) -> Result<(Vec<Edge>, Vec<Vec<usize>>, Vec<Vec<usize>>)> {
    let mut edges: Vec<Edge> = Vec::new();
    let mut map = BTreeMap::new();
    let mut ve = vec![Vec::new(); count];
    let mut vf = vec![Vec::new(); count];
    for (f, face) in faces.iter().enumerate() {
        ensure!(face.len() >= 3, "face requires at least three corners");
        let unique: std::collections::BTreeSet<_> = face.iter().collect();
        ensure!(unique.len() == face.len(), "repeated vertex in polygon");
        for (i, &a) in face.iter().enumerate() {
            let b = face[(i + 1) % face.len()];
            ensure!(
                (a as usize) < count && (b as usize) < count && a != b,
                "invalid polygon edge"
            );
            vf[a as usize].push(f);
            let key = (a.min(b), a.max(b));
            let e = *map.entry(key).or_insert_with(|| {
                let index = edges.len();
                edges.push(Edge {
                    a: a as usize,
                    sharp: sharp.get(&key).copied().unwrap_or(0.),
                    ..Default::default()
                });
                ve[a as usize].push(index);
                ve[b as usize].push(index);
                index
            });
            ensure!(edges[e].faces.len() < 2, "non-manifold subdivision edge");
            // Manifold faces traverse a shared edge in opposite directions.
            ensure!(
                edges[e].faces.is_empty() || edges[e].a != a as usize,
                "inconsistent subdivision face winding"
            );
            edges[e].faces.push(f);
        }
    }
    for (i, edge) in edges.iter_mut().enumerate() {
        if edge.faces.len() == 1 {
            edge.sharp = f32::INFINITY;
        }
        ensure!(ve[edge.a].contains(&i), "invalid subdivision topology");
    }
    for i in 0..count {
        let boundary = ve[i].iter().filter(|&&e| edges[e].faces.len() == 1).count();
        ensure!(
            !ve[i].is_empty() && (boundary == 0 || boundary == 2),
            "isolated or non-manifold subdivision vertex"
        );
        // A closed vertex fan must be connected; reject bow-ties as well as nonmanifold edges.
        let mut fan = std::collections::BTreeSet::from([vf[i][0]]);
        loop {
            let old = fan.len();
            for &e in &ve[i] {
                if edges[e].faces.iter().any(|f| fan.contains(f)) {
                    fan.extend(edges[e].faces.iter().copied());
                }
            }
            if old == fan.len() {
                break;
            }
        }
        ensure!(
            fan.len() == vf[i].len(),
            "non-manifold subdivision vertex fan"
        );
    }
    Ok((edges, ve, vf))
}

impl Plan {
    /// Exact uniform Catmull–Clark/Bilinear topology sizes, before expansion.
    /// The GPU and OpenSubdiv both address vertices/indices with u32 indices.
    pub fn refined_counts(count: usize, indices: &[u32], sizes: Option<&[u32]>, levels: u32) -> Result<(u64, u64)> {
        let mut edges = std::collections::BTreeSet::new();
        let sizes = sizes.map(Vec::from).unwrap_or_else(|| vec![3; indices.len()/3]);
        let mut at = 0usize;
        for &size in &sizes {
            ensure!(size >= 3 && at.checked_add(size as usize).is_some_and(|end| end <= indices.len()), "invalid subdivision face sizes");
            let face = &indices[at..at+size as usize];
            for i in 0..face.len() {
                let (a,b)=(face[i],face[(i+1)%face.len()]);
                edges.insert((a.min(b),a.max(b)));
            }
            at += size as usize;
        }
        ensure!(at == indices.len(), "face sizes do not cover indices");
        let (mut vertices, mut edges, mut faces, mut corners) = (count as u64, edges.len() as u64, sizes.len() as u64, indices.len() as u64);
        let mut triangles = sizes.iter().map(|&n| u64::from(n-2)*3).sum::<u64>();
        for _ in 0..levels {
            vertices = vertices.checked_add(edges).and_then(|v|v.checked_add(faces)).ok_or_else(||anyhow::anyhow!("subdivision topology size overflow"))?;
            edges = edges*2+corners;
            faces = corners;
            corners = corners.checked_mul(4).ok_or_else(||anyhow::anyhow!("subdivision topology size overflow"))?;
            triangles = faces*6;
            ensure!(vertices <= u64::from(u32::MAX) && triangles <= u64::from(u32::MAX),
                "subdivision needs {vertices} vertices and {triangles} triangle indices; topology/draw u32 index limit is {}", u32::MAX);
        }
        Ok((vertices, triangles))
    }
    /// `creases` are OpenSubdiv/USD sharpness values; 10 or greater is infinitely sharp.
    pub fn new(
        count: usize,
        indices: &[u32],
        sizes: Option<&[u32]>,
        creases: &[([u32; 2], f32)],
        levels: u32,
        simple: bool,
        limit: bool,
    ) -> Result<Self> {
        ensure!(
            count > 0 && count <= u32::MAX as usize,
            "subdivision control vertices must fit the u32 topology index limit"
        );
        let sizes = sizes
            .map(Vec::from)
            .unwrap_or_else(|| vec![3; indices.len() / 3]);
        ensure!(
            sizes.iter().map(|n| u64::from(*n)).sum::<u64>() == indices.len() as u64,
            "face sizes do not cover indices"
        );
        let mut faces = Vec::new();
        let mut offset = 0;
        for size in sizes {
            ensure!(size >= 3, "face requires at least three corners");
            faces.push(indices[offset..offset + size as usize].to_vec());
            offset += size as usize;
        }
        let mut sharp = BTreeMap::new();
        for &(vertices, crease) in creases {
            let [a, b] = vertices;
            ensure!(
                a != b
                    && (a as usize) < count
                    && (b as usize) < count
                    && crease.is_finite()
                    && crease >= 0.0,
                "invalid crease edge"
            );
            ensure!(
                sharp
                    .insert(
                        (a.min(b), a.max(b)),
                        if crease >= 10. {
                            f32::INFINITY
                        } else {
                            crease
                        }
                    )
                    .is_none(),
                "duplicate crease edge"
            );
            ensure!(
                faces.iter().any(|face| (0..face.len()).any(|i| {
                    let x = face[i];
                    let y = face[(i + 1) % face.len()];
                    (x.min(y), x.max(y)) == (a.min(b), a.max(b))
                })),
                "crease is not a polygon edge"
            );
        }
        topology(&faces, count, &sharp)?;
        let counts: Vec<usize> = faces.iter().map(Vec::len).collect();
        let pairs: Vec<_> = creases.iter().map(|c| c.0).collect();
        // OpenSubdiv consumes scene sharpness directly.
        let weights: Vec<_> = creases.iter().map(|c| c.1).collect();
        use opensubdiv_rs::{far, sdc};
        let descriptor =
            far::TopologyDescriptor::new(count, &counts, indices).with_creases(&pairs, &weights);
        let options = sdc::Options {
            vtx_boundary_interpolation: sdc::VtxBoundaryInterpolation::EdgeOnly,
            ..Default::default()
        };
        let mut refiner = far::TopologyRefinerFactory::create(
            descriptor,
            if simple {
                sdc::SchemeType::Bilinear
            } else {
                sdc::SchemeType::Catmark
            },
            options,
        )
        .map_err(|e| anyhow::anyhow!("subdivision topology: {e:?}"))?;
        Self::refined_counts(count, indices, Some(&counts.iter().map(|&n| n as u32).collect::<Vec<_>>()), levels)?;
        if levels > 0 {
            refiner.refine_uniform(far::UniformOptions::new(levels as usize));
        }
        let last = refiner.level(levels as usize);
        ensure!(
            last.num_vertices() <= u32::MAX as usize,
            "subdivision vertices exceed the u32 topology index limit"
        );
        let stencils = if levels == 0 {
            (0..count).map(|i| vec![(i as u32, 1.)]).collect()
        } else {
            let table = if limit && !simple {
                far::StencilTableFactory::create_limit(&refiner)
            } else {
                far::StencilTableFactory::create(
                    &refiner,
                    far::StencilTableOptions {
                        generate_intermediate_levels: false,
                        generate_control_verts: false,
                    },
                )
            };
            (0..table.num_stencils())
                .map(|i| {
                    let stencil = table.stencil(i);
                    stencil
                        .indices
                        .iter()
                        .copied()
                        .zip(stencil.weights.iter().copied())
                        .collect()
                })
                .collect()
        };
        let mut triangles = Vec::new();
        let mut primitive_ids = Vec::new();
        for f in 0..last.num_faces() {
            let face = last.face_vertices(f);
            let mut original = f;
            for level in (1..=levels as usize).rev() {
                original = refiner.refinement(level).child_face_parent_face(original) as usize;
            }
            for i in 1..face.len() - 1 {
                triangles.extend([face[0], face[i], face[i + 1]]);
                primitive_ids.push(original as u32);
            }
        }
        Ok(Self {
            stencils,
            indices: triangles,
            primitive_ids,
        })
    }
    pub fn evaluate<const N: usize>(&self, values: &[[f32; N]]) -> Vec<[f32; N]> {
        self.stencils
            .iter()
            .map(|s| {
                let mut out = [0.; N];
                for &(i, w) in s {
                    for axis in 0..N {
                        out[axis] += values[i as usize][axis] * w;
                    }
                }
                out
            })
            .collect()
    }
    pub fn normals(&self, positions: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let mut normals = vec![Vec3::ZERO; positions.len()];
        for t in self.indices.chunks_exact(3) {
            let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
            let n = (Vec3::from(positions[b]) - Vec3::from(positions[a]))
                .cross(Vec3::from(positions[c]) - Vec3::from(positions[a]));
            for i in [a, b, c] {
                normals[i] += n;
            }
        }
        normals
            .into_iter()
            .map(|n| n.normalize_or_zero().to_array())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uniform_sizes_cross_old_scene_caps_and_guard_index_overflow() -> Result<()> {
        assert_eq!(Plan::refined_counts(4,&[0,1,2,3],Some(&[4]),10)?,(1_050_625,6_291_456));
        let plan=Plan::new(4,&[0,1,2,3],Some(&[4]),&[],7,true,false)?;
        assert_eq!(plan.stencils.len(),16_641);
        assert!(Plan::refined_counts(4,&[0,1,2,3],Some(&[4]),u32::MAX).unwrap_err().to_string().contains("u32 index limit"));
        Ok(())
    }

    #[test]
    fn cube_refinement_and_affine_invariance() -> Result<()> {
        let indices = [
            0, 1, 3, 2, 2, 3, 5, 4, 4, 5, 7, 6, 6, 7, 1, 0, 1, 7, 5, 3, 6, 0, 2, 4,
        ];
        let p = [
            [-1., -1., 1.],
            [1., -1., 1.],
            [-1., 1., 1.],
            [1., 1., 1.],
            [-1., 1., -1.],
            [1., 1., -1.],
            [-1., -1., -1.],
            [1., -1., -1.],
        ];
        let plan = Plan::new(8, &indices, Some(&[4; 6]), &[], 1, false, false)?;
        assert_eq!(plan.stencils.len(), 26);
        assert_eq!(plan.indices.len(), 144);
        let refined = plan.evaluate(&p);
        // Corner vertex after one Catmull–Clark step: 5/9 of each cube axis.
        assert!(refined.iter().any(|v| (v[0].abs()-5./9.).abs()<1e-6 && (v[1].abs()-5./9.).abs()<1e-6));
        for s in &plan.stencils {
            assert!((s.iter().map(|x| x.1).sum::<f32>() - 1.).abs() < 1e-5);
        }
        // Ten is OpenSubdiv's infinite sharpness; larger authored values agree.
        let edges: Vec<_> = indices.chunks_exact(4).flat_map(|face|
            (0..4).map(move |i| { let a=face[i]; let b=face[(i+1)%4]; (a.min(b),a.max(b)) })
        ).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let sharp = |value| {
            let creases: Vec<_> = edges.iter().map(|&(a,b)| ([a,b],value)).collect();
            Plan::new(8, &indices, Some(&[4;6]), &creases, 2, false, false).unwrap().evaluate(&p)
        };
        assert_eq!(sharp(10.), sharp(20.));
        for corner in p { assert!(sharp(10.).contains(&corner)); }
        let shifted: Vec<_> = p
            .iter()
            .map(|v| [v[0] + 3., v[1] + 2., v[2] - 4.])
            .collect();
        for (a, b) in plan.evaluate(&shifted).iter().zip(&refined) {
            assert!((a[0] - b[0] - 3.).abs() < 1e-5);
        }
        assert!(Plan::new(8, &indices, Some(&[4; 6]), &[([0, 7], 0.5)], 1, false, true).is_err());
        Ok(())
    }

    #[test]
    fn inconsistent_face_winding_is_rejected() {
        let mut indices = [
            0, 1, 3, 2, 2, 3, 5, 4, 4, 5, 7, 6, 6, 7, 1, 0, 1, 7, 5, 3, 6, 0, 2, 4,
        ];
        assert!(Plan::new(8, &indices, Some(&[4; 6]), &[], 2, false, true).is_ok());
        indices[..4].reverse();
        let error = Plan::new(8, &indices, Some(&[4; 6]), &[], 2, false, true)
            .err()
            .expect("reversed face accepted");
        assert!(error.to_string().contains("winding"));
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    #[test]
    fn descendants_retain_their_original_polygon_ordinal() {
        let indices = [0, 1, 4, 3, 1, 2, 5, 4];
        let plan = Plan::new(6, &indices, Some(&[4, 4]), &[], 2, true, false).unwrap();
        assert_eq!(plan.primitive_ids.len(), plan.indices.len() / 3);
        assert_eq!(plan.primitive_ids.iter().filter(|&&i| i == 0).count(), 32);
        assert_eq!(plan.primitive_ids.iter().filter(|&&i| i == 1).count(), 32);
    }
}
