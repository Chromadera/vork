//! A complete power reduction tree derived from the evaluated shared lights.
//! Slot zero is unused; leaves start at half the allocation. Shader traversal
//! uses the same child sums for selection and its returned event probability.
use super::Light;

pub(crate) fn node_count(lights: usize) -> usize {
    lights.max(1).next_power_of_two() * 2
}

pub(crate) fn powers(lights: &[Light]) -> Vec<f32> {
    let weights: Vec<f64> = lights
        .iter()
        .map(|light| {
            light.radiance[..3]
                .iter()
                .zip(crate::constants::LUMINANCE_REC709)
                .map(|(value, luma)| f64::from(*value) * f64::from(luma))
                .sum::<f64>()
                .max(0.)
        })
        .collect();
    // Scaling preserves probabilities and prevents overflow when many bright
    // emitters are reduced. No emitter properties live in this derived table.
    let scale = weights.iter().copied().fold(0., f64::max);
    let mut tree = vec![0.; node_count(lights.len())];
    let leaves = tree.len() / 2;
    if scale > 0. {
        for (leaf, power) in tree[leaves..].iter_mut().zip(weights) {
            *leaf = (power / scale) as f32;
        }
        for node in (1..leaves).rev() {
            tree[node] = tree[node * 2] + tree[node * 2 + 1];
        }
    }
    tree
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
            uniform: buffer("unused uniform", 16),
            lights: buffer("shared lights", 2 * std::mem::size_of::<Light>()),
            path_light_tree: Some(buffer("power distribution", 16)),
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
        let tree: Vec<f32> = changes.bytes[*offset as usize..(*offset + *size) as usize]
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(tree[2], 1.);
        assert!((tree[3] - 1. / 3.).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn reduction_handles_padding_dark_lights_and_bright_animated_values() {
        let mut lights = vec![Light::zeroed(); 5];
        for (light, power) in lights.iter_mut().zip([0., 1., 2., 0., 5.]) {
            light.radiance = [power, power, power, 0.];
        }
        let tree = powers(&lights);
        assert_eq!(tree.len(), 16);
        assert_eq!(&tree[13..], &[0.; 3]);
        for node in 1..8 {
            assert_eq!(tree[node], tree[node * 2] + tree[node * 2 + 1]);
        }
        assert!((tree[1] - 1.6).abs() < 1e-6);
        lights[0].radiance = [f32::MAX; 4];
        assert!(powers(&lights).iter().all(|p| p.is_finite()));
        lights.fill(Light::zeroed());
        assert!(powers(&lights).iter().all(|p| *p == 0.));
        assert_eq!(powers(&[]), vec![0.; 2]);
    }
}
