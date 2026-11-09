//! Retained spatial proposal derived from the evaluated analytic light buffer.
use super::{Light, tree::{Tree, Node, analytic_sources}};

pub(crate) fn node_count(lights: usize) -> usize { lights.saturating_mul(2).saturating_sub(1).max(1) }

pub(crate) fn nodes(lights: &[Light]) -> anyhow::Result<Vec<Node>> {
    // path_sampler.sample_unit supplies 23-bit midpoint source coordinates.
    anyhow::ensure!(lights.len() <= (1 << 23), "analytic light tree exceeds the path sampler source-coordinate capacity");
    let tree = Tree::build(&analytic_sources(lights))?;
    Ok(if tree.nodes.is_empty() { vec![bytemuck::Zeroable::zeroed()] } else { tree.nodes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn changed_light_values_update_the_distribution_through_the_upload_tracker()
    -> anyhow::Result<()> {
        let Some(r) = crate::test_renderer()? else {
            return Ok(());
        };
        let buffer = |label, size| {
            crate::init(
                &r.device,
                label,
                &vec![0; size],
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            )
        };
        let lighting = crate::view::ViewLighting {
            active: true,
            uniform: buffer("unused uniform", 16),
            lights: buffer("shared lights", 2 * std::mem::size_of::<Light>()),
            path_light_tree: Some(buffer("spatial distribution", node_count(2) * std::mem::size_of::<Node>())),
            cluster_config: buffer("unused config", 16),
            cluster_lights: buffer("unused cluster", 4),
            clusters: None,
        };
        let mut lights = vec![Light::zeroed(); 2];
        lights[0].radiance = [1.; 4];
        lights[1].radiance = [3.; 4];
        let mut changes = crate::values::ChangeTracker::default();
        lighting.upload_lights(&mut changes, &lights)?;
        assert_eq!(changes.uploads.len(), 2);
        changes.uploads.clear();
        changes.bytes.clear();
        lighting.upload_lights(&mut changes, &lights)?;
        assert!(
            changes.uploads.is_empty(),
            "unchanged energies were uploaded again"
        );
        lights.swap(0, 1);
        lighting.upload_lights(&mut changes, &lights)?;
        assert_eq!(
            changes.uploads.len(),
            2,
            "animated powers left a stale distribution"
        );
        let (_, _, offset, size) = changes.uploads.last().unwrap();
        let tree: Vec<Node> = changes.bytes[*offset as usize..(*offset + *size) as usize]
            .chunks_exact(std::mem::size_of::<Node>())
            .map(bytemuck::pod_read_unaligned)
            .collect();
        let leaf = |identity| tree.iter().find(|n| n.links[0] == u32::MAX && n.source[1] == identity).unwrap();
        assert_eq!(leaf(0).lower_power[3], 1.);
        assert!((leaf(1).lower_power[3] - 1. / 3.).abs() < 1e-6);
        changes.uploads.clear(); changes.bytes.clear();
        lights[0].position[0] = 7.;
        lighting.upload_lights(&mut changes, &lights)?;
        assert_eq!(changes.uploads.len(), 2, "moving lights left stale spatial bounds");
        Ok(())
    }

    #[test]
    fn spatial_nodes_handle_empty_dark_and_hdr_guides() -> anyhow::Result<()> {
        assert_eq!(nodes(&[])?.len(), node_count(0));
        let mut lights = vec![Light::zeroed(); 5];
        for (i, light) in lights.iter_mut().enumerate() {
            light.position = [i as f32, 0., 0., 1.];
            light.radiance = [if i == 0 { f32::MAX } else { 0. }; 4];
        }
        let tree = nodes(&lights)?;
        assert_eq!(tree.len(), node_count(lights.len()));
        assert!(tree.iter().all(|n| n.lower_power.iter().chain(n.upper_infinite.iter()).all(|p| p.is_finite())));
        assert_eq!(tree.iter().filter(|n| n.links[0] == u32::MAX).count(), lights.len());
        Ok(())
    }
}
