//! Native environment installation and per-frame values.
use super::*;
use ::vork_scene::v2;

#[derive(Debug)]
pub(crate) struct Settings {
    /// Preorder index of the environment light node.
    pub source: Option<usize>,
    pub material: Option<String>,
    pub medium: Option<String>,
}
impl Settings {
    pub fn has_radiance(&self) -> bool {
        self.material.is_some()
    }
    pub fn from_scene(scene: &v2::Scene) -> Result<Self> {
        fn visit(
            nodes: &[v2::Node],
            next: &mut usize,
            found: &mut Option<Settings>,
            scene: &v2::Scene,
            first: &mut Option<String>,
        ) -> Result<()> {
            for node in nodes {
                let index = *next;
                *next += 1;
                if let v2::NodeKind::Light {
                    shape: v2::LightShape::Environment,
                    medium,
                    material,
                    ..
                } = &node.kind
                {
                    let label = format!("node {index} ({:?})", node.name);
                    if let Some(first) = first.as_ref() {
                        bail!("only one environment is supported; found {first} and {label}");
                    }
                    *first = Some(label);
                    *found = Some(Settings {
                        source: Some(index),
                        material: material.clone(),
                        medium: medium.clone(),
                    });
                }
                visit(&node.children, next, found, scene, first)?;
            }
            Ok(())
        }
        let mut found = None;
        visit(&scene.nodes, &mut 0, &mut found, scene, &mut None)?;
        Ok(found.unwrap_or(Settings {
            source: None,
            material: None,
            medium: None,
        }))
    }
}

pub(crate) struct Sample {
    pub color: [f32; 3],
    pub intensity: f32,
    pub rotation: Mat4,
}
impl Sample {
    pub fn constant(&self, image: bool) -> [f32; 3] {
        if image {
            [0.; 3]
        } else {
            self.color.map(|v| v * self.intensity)
        }
    }
}
pub(crate) fn sample<N: std::ops::Index<usize, Output = EvalNode> + ?Sized>(
    node: Option<usize>,
    nodes: &N,
    worlds: &[Mat4],
) -> Result<Sample> {
    let Some(index) = node else {
        return Ok(Sample {
            color: v2::ENVIRONMENT_COLOR,
            intensity: v2::LightShape::Environment.default_energy(),
            rotation: Mat4::IDENTITY,
        });
    };
    // Nodes are this frame's sampled values; tracks were applied by the frame sampler.
    let NodeKind::Light {
        shape: v2::LightShape::Environment,
        color,
        energy: intensity,
        material,
        ..
    } = &nodes[index].node.kind
    else {
        bail!("environment node lost its installed light kind");
    };
    let (color, intensity) = (
        color.unwrap_or(
            v2::LightShape::Environment.default_color(material.is_some()),
        ),
        intensity.unwrap_or(v2::LightShape::Environment.default_energy()),
    );
    ensure!(
        color.iter().all(|v| v.is_finite() && *v >= 0.) && intensity.is_finite() && intensity >= 0.,
        "environment color and intensity must be finite and nonnegative"
    );
    let frame = if crate::buffers::node_collapsed(nodes, index) || crate::collapsed(worlds[index]) {crate::values::node_view(index, nodes, worlds)?.inverse()} else {worlds[index]};
    let (_, rotation, _) = frame.to_scale_rotation_translation();
    Ok(Sample {
        color,
        intensity,
        rotation: Mat4::from_quat(rotation.normalize().conjugate()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_environment_keeps_children_and_samples_full_rotation() -> Result<()> {
        let q = glam::Quat::from_rotation_x(0.6) * glam::Quat::from_rotation_z(0.3);
        let input: v2::Scene = vork_scene::v2::from_test_value(
            json!({"version":2,"nodes":[{"type":"light","shape":"environment","name":"sky","color":[0.4,0.6,0.8],"energy":2,"rotation":q.to_array(),"children":[{"type":"camera","name":"cam","material":{"program":"std/camera","inputs":{"projection":"persp"}}}]}]}),
        )?;
        let node = input.nodes[0].without_children();
        let world = node.transform.matrix();
        let nodes = vec![EvalNode { camera: None,
            node,
            parent: None,
            world,
            dynamic: false,
            embedded: None,
        }];
        let value = sample(Some(0), &nodes, &[world])?;
        assert!((value.rotation.transform_vector3(q * Vec3::Y) - Vec3::Y).length() < 1e-6);
        assert_eq!(value.constant(false), [0.8, 1.2, 1.6]);
        assert_eq!(Settings::from_scene(&input)?.source, Some(0));
        Ok(())
    }
    #[test]
    fn environment_defaults_and_duplicate_boundary() -> Result<()> {
        let default = sample(None, &Vec::<EvalNode>::new(), &[])?;
        assert_eq!(default.constant(false), v2::ENVIRONMENT_COLOR);
        let input = vork_scene::v2::from_test_value(
            json!({"version":2,"nodes":[{"type":"light","shape":"environment","name":"world_a"},{"type":"node","children":[{"type":"light","shape":"environment","name":"world_b","energy":0}]}]}),
        )?;
        let error = Settings::from_scene(&input).unwrap_err().to_string();
        for expected in ["only one", "world_a", "world_b", "node 0", "node 2"] {
            assert!(error.contains(expected), "{error}");
        }
        assert!(!error.contains("raster"));
        Ok(())
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn extracted_sun_follows_tilted_parent() -> Result<()> {
        let tilt = glam::Quat::from_rotation_x(0.7) * glam::Quat::from_rotation_z(0.2);
        let local = glam::Quat::from_rotation_y(0.4);
        let mut parent: Node = vork_scene::v2::from_test_value(
            serde_json::json!({"type":"light","shape":"environment","color":[1,1,1],"energy":1}),
        )?;
        parent.transform.rotation = Some(tilt.to_array());
        let mut sun: Node = vork_scene::v2::from_test_value(
            serde_json::json!({"type":"light","shape":"sun","shadowed_by":[],"energy":1}),
        )?;
        sun.transform.rotation = Some(local.to_array());
        let worlds = [Mat4::from_quat(tilt), Mat4::from_quat(tilt * local)];
        let nodes = vec![
            EvalNode { camera: None,
                node: parent,
                parent: None,
                world: worlds[0],
                dynamic: false,
                embedded: None,
            },
            EvalNode { camera: None,
                node: sun,
                parent: Some(0),
                world: worlds[1],
                dynamic: false,
                embedded: None,
            },
        ];
        let plan = LightPlan::logical(&nodes, &wgpu::Limits::default(), false)?;
        let lights = plan.evaluate(
            &nodes,
            &worlds,
            &lights::links::Links::new(&nodes)?,
        )?;
        assert!(
            (Vec3::from_slice(&lights[0].direction[..3]) - (tilt * local * Vec3::NEG_Z)).length()
                < 1e-6
        );
        Ok(())
    }
}
