//! Shared numeric curve sampling for authored v2 tracks and imported glTF clips.
use anyhow::{Context, Result, ensure};
use glam::Quat;
use vork_scene::v2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interpolation {
    Step,
    Linear,
    Cubic,
}
/// Cubic samples use glTF's [incoming tangent, value, outgoing tangent] packing.
pub(crate) struct Curve {
    pub times: Vec<f32>,
    pub values: Vec<f32>,
    pub width: usize,
    pub interpolation: Interpolation,
    pub rotation: bool,
    /// Rotation keyed as XYZ Euler angles: each angle interpolates on its own
    /// (so a spin past 180° is two keys) and converts to a quaternion per sample.
    /// `values` then holds three angles per key; samples are still quaternions.
    pub euler: bool,
}
impl Curve {
    pub fn duration(&self) -> f32 {
        *self.times.last().unwrap()
    }
    pub fn sample(&self, time: f32, out: &mut Vec<f32>) {
        let upper = self.times.partition_point(|v| *v <= time);
        let a = upper.saturating_sub(1);
        let b = upper.min(self.times.len() - 1);
        let dt = self.times[b] - self.times[a];
        let u = if dt > 0. {
            ((time - self.times[a]) / dt).clamp(0., 1.)
        } else {
            0.
        };
        let cubic = self.interpolation == Interpolation::Cubic;
        let stride = if self.euler { 3 } else { self.width };
        let value = |key: usize, part: usize, k: usize| {
            self.values[(key * if cubic { 3 } else { 1 } + part) * stride + k]
        };
        if self.euler {
            let angle = |k: usize| if cubic {
                let (u2, u3) = (u * u, u * u * u);
                (2. * u3 - 3. * u2 + 1.) * value(a, 1, k) + (u3 - 2. * u2 + u) * dt * value(a, 2, k)
                    + (-2. * u3 + 3. * u2) * value(b, 1, k) + (u3 - u2) * dt * value(b, 0, k)
            } else if self.interpolation == Interpolation::Step {
                value(a, 0, k)
            } else {
                value(a, 0, k) * (1. - u) + value(b, 0, k) * u
            };
            let euler = v2::Rotation::Euler { euler: [angle(0), angle(1), angle(2)] };
            out.clear();
            out.extend(euler.quaternion());
            return;
        }
        out.resize(self.width, 0.);
        if self.rotation && self.interpolation == Interpolation::Linear {
            let q = |i| Quat::from_array(std::array::from_fn(|k| value(i, 0, k))).normalize();
            out.copy_from_slice(&q(a).slerp(q(b), u).normalize().to_array());
            return;
        }
        for (k, out) in out.iter_mut().enumerate() {
            *out = if cubic {
                let u2 = u * u;
                let u3 = u2 * u;
                (2. * u3 - 3. * u2 + 1.) * value(a, 1, k)
                    + (u3 - 2. * u2 + u) * dt * value(a, 2, k)
                    + (-2. * u3 + 3. * u2) * value(b, 1, k)
                    + (u3 - u2) * dt * value(b, 0, k)
            } else if self.interpolation == Interpolation::Step {
                value(a, 0, k)
            } else {
                value(a, 0, k) * (1. - u) + value(b, 0, k) * u
            };
        }
        if self.rotation {
            let q = Quat::from_array(out.as_slice().try_into().unwrap());
            let q = if q.length_squared() > 0. {
                q
            } else {
                Quat::from_array(std::array::from_fn(|k| value(a, usize::from(cubic), k)))
            };
            out.copy_from_slice(&q.normalize().to_array());
        }
    }

    /// Compile explicit glTF CUBICSPLINE derivatives once.
    pub fn authored(
        track: &v2::Track,
        rotation: bool,
        times: Vec<f32>,
        values: impl AsRef<[serde_json::Value]>,
    ) -> Result<Self> {
        let values = values.as_ref();
        ensure!(!values.is_empty(), "track needs values");
        let euler = rotation && values[0].get("euler").is_some();
        let angles = |value: &serde_json::Value| -> Result<Vec<f32>> {
            let angles = value.get("euler").context("a rotation track's keys are all [x,y,z,w] or all {\"euler\": [x,y,z]}")?;
            let angles = components(angles)?;
            ensure!(angles.len() == 3, "euler rotation needs [x,y,z] radians");
            Ok(angles)
        };
        let width = if euler { 3 } else { components(&values[0])?.len() };
        let mut flat = Vec::with_capacity(values.len() * width);
        for value in values {
            let value = if euler { angles(value)? } else { components(value)? };
            ensure!(value.len() == width, "track value shape mismatch");
            flat.extend(value);
        }
        if euler {
            let mut curve = Self::numeric(track, false, times, flat, 3)?;
            curve.rotation = true;
            curve.euler = true;
            curve.width = 4;
            return Ok(curve);
        }
        Self::numeric(track, rotation, times, flat, width)
    }
    fn numeric(
        track: &v2::Track,
        rotation: bool,
        times: Vec<f32>,
        values: Vec<f32>,
        width: usize,
    ) -> Result<Self> {
        let path = v2::display_path(&track.path);
        ensure!(
            width > 0
                && !times.is_empty()
                && values.len() == times.len() * width * if matches!(track.interpolation, v2::Interpolation::Cubic) { 3 } else { 1 }
                && times.iter().all(|v| v.is_finite())
                && times.windows(2).all(|v| v[0] < v[1])
                && values.iter().all(|v| v.is_finite()),
            "invalid numeric track {path}"
        );
        ensure!(
            !rotation
                || (width == 4
                    && values
                        .chunks_exact(width)
                        .enumerate()
                        .filter(|(i, _)| !matches!(track.interpolation, v2::Interpolation::Cubic) || i % 3 == 1)
                        .all(|(_, v)| Quat::from_slice(v).length_squared() > 0.)),
            "invalid quaternion track"
        );
        let interpolation = match track.interpolation {
            v2::Interpolation::Step => Interpolation::Step,
            v2::Interpolation::Linear => Interpolation::Linear,
            v2::Interpolation::Cubic => Interpolation::Cubic,
        };
        Ok(Self {
            times,
            values,
            width,
            interpolation,
            rotation,
            euler: false,
        })
    }
}

fn components(value: &serde_json::Value) -> Result<Vec<f32>> {
    let values = if let Some(array) = value.as_array() {
        array
            .iter()
            .map(|v| v.as_f64().context("numeric track component"))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![value.as_f64().context("numeric track value")?]
    };
    ensure!(
        values.iter().all(|v| (*v as f32).is_finite()),
        "track values must be finite"
    );
    Ok(values.into_iter().map(|v| v as f32).collect())
}

/// How an additive layer combines with the value below it (Blender NLA Combine).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Combine {
    /// Offsets add: translation, weights, scalars.
    Add,
    /// Scale multiplies: base × sample^weight.
    Multiply,
    /// Quaternions post-multiply: base × slerp(identity, sample, weight).
    Rotation,
}
/// Replace layers mix toward the sample (three.js AnimationMixer / NLA Replace);
/// additive layers follow Blender NLA Combine for the value's kind.
pub(super) fn blend(base: &mut [f32], sample: &[f32], weight: f32, additive: bool, kind: Combine) {
    if kind == Combine::Rotation {
        let current = Quat::from_slice(base);
        let sampled = Quat::from_slice(sample).normalize();
        let result = if additive {
            current * Quat::IDENTITY.slerp(sampled, weight)
        } else {
            current.slerp(sampled, weight)
        };
        base.copy_from_slice(&result.normalize().to_array());
        return;
    }
    for (base, sample) in base.iter_mut().zip(sample) {
        *base = match (additive, kind) {
            (false, _) => *base + weight * (sample - *base),
            (true, Combine::Multiply) => *base * sample.signum() * sample.abs().powf(weight),
            (true, _) => *base + weight * sample,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_buffer_times_are_checked_after_decode() -> Result<()> {
        let base = std::env::temp_dir().join(format!("vork-track-times-{}", std::process::id()));
        std::fs::create_dir_all(&base)?;
        let result = (|| -> Result<()> {
            {
                for (times, valid) in [
                    (vec![], false),
                    (vec![0., 1., 2.], false),
                    (vec![-1., 1.], false),
                    (vec![0., f32::NAN], false),
                    (vec![0., f32::INFINITY], false),
                    (vec![1., 0.], false),
                    (vec![0., 0.], false),
                    (vec![0., 1.], true),
                ] {
                    let bytes: Vec<_> = times.iter().flat_map(|t| t.to_le_bytes()).collect();
                    std::fs::write(base.join("times.bin"), bytes)?;
                    // Buffer-backed keys are numeric; names and switches are inline pairs.
                    let scene: v2::Scene = vork_scene::v2::from_test_value(serde_json::json!({
                        "version":2,"nodes":[{"type":"camera","name":"cam","material":"optics"}],
                        "materials":{"optics":{"program":"std/camera"}},
                        "buffers":{"t":{"type":"f32","file":"times.bin"},"v":{"type":"f32","data":[0,1]}},
                        "animation":[{"tracks":[{"path":["materials","optics","inputs","lens"],"keys":{"times":"t","values":"v"},"interpolation":"STEP"}]}]
                    }))?;
                    scene.validate()?;
                    assert_eq!(
                        Plan::compile(&scene, &base).is_ok(),
                        valid,
                        "times={times:?}"
                    );
                }
            }
            Ok(())
        })();
        std::fs::remove_dir_all(base)?;
        result
    }

    #[test]
    fn cubic_derivatives_are_scaled_by_segment_duration() -> Result<()> {
        let track: v2::Track = vork_scene::v2::from_test_value(
            serde_json::json!({"path":["materials","optics","inputs","lens"],"keys":[[0,0,0,2],[2,2,0,0]],"interpolation":"CUBICSPLINE"}),
        )?;
        let v2::Values::Inline(values) = &track.values else { unreachable!() };
        let curve = Curve::authored(&track, false, vec![0., 2.], values)?;
        let mut value = vec![];
        curve.sample(1., &mut value);
        assert_eq!(value, [1.5]);
        curve.sample(-1., &mut value);
        assert_eq!(value, [0.]);
        curve.sample(3., &mut value);
        assert_eq!(value, [2.]);
        Ok(())
    }
    #[test]
    fn additive_layers_combine_like_nla_and_replacement_is_ordered() {
        let x = Quat::from_rotation_x(0.7);
        let y = Quat::from_rotation_y(0.4);
        let mut base = x.to_array();
        blend(&mut base, &y.to_array(), 0.5, true, Combine::Rotation);
        // The offset turns about the current local axes (post-multiply).
        assert!(
            Quat::from_array(base)
                .dot(x * Quat::IDENTITY.slerp(y, 0.5))
                .abs()
                > 0.99999
        );
        let mut base = [10.];
        blend(&mut base, &[20.], 0.5, false, Combine::Add);
        blend(&mut base, &[4.], 0.5, true, Combine::Add);
        assert_eq!(base, [17.]);
        let mut scale = [2.];
        blend(&mut scale, &[4.], 0.5, true, Combine::Multiply);
        assert_eq!(scale, [4.]);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sample {
    Numeric(Vec<f32>),
    String(std::sync::Arc<str>),
    Bool(bool),
}
#[derive(Clone)]
pub(crate) enum Channel {
    Numeric(std::sync::Arc<Curve>),
    Discrete {
        times: Vec<f32>,
        values: Vec<Sample>,
    },
}
#[derive(Clone, Copy)]
pub(crate) struct AssetTarget {
    pub asset: usize,
    pub node: usize,
    pub property: gltf::animation::Property,
    pub component: Option<usize>,
}
#[derive(Clone)]
pub(crate) struct PropertyTrack {
    /// The authored target; None for a glTF clip channel (asset targets only).
    pub target: Option<v2::TrackTarget>,
    channel: Channel,
    weight_target: Option<usize>,
    asset_targets: Vec<AssetTarget>,
    asset_visibility: Option<(usize, usize)>,
    node_targets: Vec<NodeBinding>,
    material_targets: Option<Vec<usize>>,
}
#[derive(Clone)]
pub(crate) struct CompiledLayer {
    pub source: v2::Layer,
    pub tracks: Vec<PropertyTrack>,
    span: Option<(f32, f32)>,
}
/// One immutable ordered layer plan. glTF import appends channels to these layers.
#[derive(Clone)]
pub(crate) struct MaterialBinding {
    pub slot: usize,
    pub parameter: usize,
    /// Reflected input storage; slot is an absolute word address when present.
    pub storage: Option<vork_slang::Type>,
    pub enum_tokens: Option<Vec<String>>,
    pub default: Sample,
    /// Instance time offset of this record copy: it is written at time + offset.
    pub offset: f32,
}
#[derive(Clone)]
pub(crate) struct Samples {
    values: Vec<Sample>,
    weights: Vec<f32>,
    layer_weights: Vec<f32>,
    pub render: Option<v2::Render>,
    material_values: Vec<Sample>,
}
#[derive(Clone, Default)]
pub(crate) struct Plan {
    pub layers: Vec<CompiledLayer>,
    reset_bindings: Vec<NodeBinding>,
    material_bindings: Vec<MaterialBinding>,
    material_offsets: Vec<f32>,
    cameras: std::collections::BTreeMap<String, usize>,
    camera_events: Vec<(f32, usize)>,
    camera_start: Option<usize>,
    duration: f32,
}
impl Plan {
    #[cfg(test)]
    pub fn compile(scene: &v2::Scene, base: &std::path::Path) -> Result<Self> {
        Self::compile_with_buffers(scene, base, None)
    }
    pub fn compile_with_buffers(
        scene: &v2::Scene,
        base: &std::path::Path,
        workspace: Option<&std::collections::BTreeMap<String, crate::buffers::AttributeData>>,
    ) -> Result<Self> {
        use std::collections::HashMap;
        let names: HashMap<_, _> = scene
            .animation
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.name.as_ref().map(|n| (n.as_str(), i)))
            .collect();
        let targets = scene.track_targets()?;
        let mut layers = Vec::new();
        for (source, targets) in scene.animation.iter().zip(targets) {
            ensure!(
                source.weight.is_finite() && source.offset.is_finite() && source.speed.is_finite(),
                "layer weight, offset and speed must be finite"
            );
            let mut tracks = Vec::new();
            let mut span: Option<(f32, f32)> = source.source_span;
            for (track, target) in source.tracks.iter().flatten().zip(targets) {
                use v2::{NodeProperty, TrackTarget};
                let rotation = matches!(
                    target,
                    TrackTarget::Node {
                        property: NodeProperty::Rotation,
                        ..
                    } | TrackTarget::Imported {
                        property: NodeProperty::Rotation,
                        ..
                    }
                );
                let times = match &track.times {
                    v2::Times::Inline(times) => times.clone(),
                    v2::Times::Buffer(name) => {
                        let (times, width) = numeric_buffer(scene, name, base, workspace)?;
                        ensure!(width == 1, "time buffer must be scalar");
                        times
                    }
                };
                ensure!(
                    !times.is_empty()
                        && times.iter().all(|t| t.is_finite() && *t >= 0.)
                        && times.windows(2).all(|w| w[0] < w[1]),
                    "track {}: times must be non-empty, finite, >= 0 and strictly increasing",
                    v2::display_path(&track.path)
                );
                let weight_target = if let TrackTarget::LayerWeight { layer } = &target {
                    Some(
                        *names
                            .get(layer.as_str())
                            .context("unknown animation layer weight target")?,
                    )
                } else {
                    None
                };
                let channel = match &track.values {
                    v2::Values::Inline(values)
                        if values
                            .first()
                            .is_some_and(|v| v.is_string() || v.is_boolean()) =>
                    {
                        ensure!(
                            values.len() == times.len(),
                            "track {}: discrete values must match the time count",
                            v2::display_path(&track.path)
                        );
                        let strings = values[0].is_string();
                        let values = values
                            .iter()
                            .map(|v| {
                                if strings {
                                    let text = v.as_str().context("mixed track value types")?;
                                    Ok(Sample::String(text.into()))
                                } else {
                                    v.as_bool()
                                        .map(Sample::Bool)
                                        .context("mixed track value types")
                                }
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Channel::Discrete {
                            times: times.clone(),
                            values,
                        }
                    }
                    values => {
                        let curve = match values {
                            v2::Values::Inline(values) => {
                                Curve::authored(track, rotation, times.clone(), values)?
                            }
                            v2::Values::Buffer(name) => {
                                let (values, width) = numeric_buffer(scene, name, base, workspace)?;
                                Curve::numeric(track, rotation, times.clone(), values, width)?
                            }
                        };
                        Channel::Numeric(std::sync::Arc::new(curve))
                    }
                };
                span = Some(span.map_or((times[0], *times.last().unwrap()), |(a, b)| {
                    (a.min(times[0]), b.max(*times.last().unwrap()))
                }));
                tracks.push(PropertyTrack {
                    target: Some(target),
                    channel,
                    weight_target,
                    asset_targets: vec![],
                    asset_visibility: None,
                    node_targets: vec![],
                    material_targets: None,
                });
            }
            layers.push(CompiledLayer {
                source: v2::Layer {
                    source_span: source.source_span,
                    name: source.name.clone(),
                    tracks: None,
                    clip: source.clip.clone(),
                    weight: source.weight,
                    blend: source.blend.clone(),
                    offset: source.offset,
                    speed: source.speed,
                    looping: source.looping,
                },
                tracks,
                span,
            });
        }
        Ok(Self {
            layers,
            duration: scene.render.duration,
            camera_events: vec![],
            camera_start: None,
            reset_bindings: vec![],
            material_bindings: vec![],
            material_offsets: vec![],
            cameras: Default::default(),
        })
    }
    /// Clip declarations retain their global layer indices in the compiled plan.
    pub fn clips_for_asset<'a>(
        &'a self,
        root: &'a str,
    ) -> impl Iterator<Item = (usize, &'a v2::Clip)> + 'a {
        self.layers
            .iter()
            .enumerate()
            .filter_map(move |(index, layer)| {
                let clip = layer.source.clip.as_ref()?;
                (clip.target == root)
                    .then_some((index, clip))
            })
    }

    pub fn add_clip_curve(
        &mut self,
        layer: usize,
        curve: std::sync::Arc<Curve>,
        target: AssetTarget,
    ) {
        let span = (0., curve.duration());
        let layer = &mut self.layers[layer];
        layer.span = Some(
            layer
                .span
                .map_or(span, |(a, b)| (a.min(span.0), b.max(span.1))),
        );
        layer.tracks.push(PropertyTrack {
            target: None,
            channel: Channel::Numeric(curve),
            weight_target: None,
            asset_targets: vec![target],
            asset_visibility: None,
            node_targets: vec![],
            material_targets: None,
        });
    }
    pub fn bind_asset(&mut self, doc: &gltf::Document, root: &str, asset: usize) -> Result<bool> {
        use v2::{NodeProperty, TrackTarget};
        let mut any = false;
        for track in self.layers.iter_mut().flat_map(|l| &mut l.tracks) {
            let Some(TrackTarget::Imported {
                asset: name,
                node,
                property,
            }) = &track.target
            else {
                continue;
            };
            if name != root {
                continue;
            }
            let path = || format!("[\"nodes\",{name:?},\"nodes\",{node:?}]");
            let imported = if let Some(index) = node.strip_prefix('#').and_then(|index| index.parse::<usize>().ok()) {
                doc.nodes()
                    .nth(index)
                    .with_context(|| {
                        format!("track {}: imported node index out of range", path())
                    })?
            } else {
                let nodes: Vec<_> = doc.nodes().filter(|n| n.name() == Some(node)).collect();
                ensure!(
                    nodes.len() == 1,
                    "track {}: segment 3: {}",
                    path(),
                    if nodes.is_empty() {
                        ::vork_scene::v2::diagnostics::unknown(
                            "imported node",
                            node,
                            doc.nodes().filter_map(|n| n.name()),
                        )
                    } else {
                        format!(
                            "{} imported nodes are named {node:?}; use #index",
                            nodes.len()
                        )
                    }
                );
                nodes[0].clone()
            };
            if matches!(property, NodeProperty::Visibility(v2::Ray::All)) {
                ensure!(matches!(&track.channel, Channel::Discrete {values,..} if matches!(values[0], Sample::Bool(_))), "imported visible requires boolean values");
                track.asset_visibility = Some((asset, imported.index()));
                any = true;
                continue;
            }
            let (property, component, width) = match property {
                NodeProperty::Translation => (gltf::animation::Property::Translation, None, 3),
                NodeProperty::Rotation => (gltf::animation::Property::Rotation, None, 4),
                NodeProperty::Scale => (gltf::animation::Property::Scale, None, 3),
                NodeProperty::MorphWeight(name) => {
                    let mesh = imported.mesh().context("morph track target needs a mesh")?;
                    let count = mesh
                        .primitives()
                        .next()
                        .context("morph track needs primitives")?
                        .morph_targets()
                        .len();
                    let extras: serde_json::Value = mesh
                        .extras()
                        .as_ref()
                        .map(|v| serde_json::from_str(v.get()))
                        .transpose()?
                        .unwrap_or_default();
                    let component = if let Some(index) = name.strip_prefix('#') {
                        index.parse::<usize>().context("invalid morph index")?
                    } else {
                        extras["targetNames"]
                            .as_array()
                            .and_then(|names| names.iter().position(|v| v.as_str() == Some(name)))
                            .with_context(|| {
                                format!(
                                    "unknown morph {name}; available: {}",
                                    extras["targetNames"]
                                )
                            })?
                    };
                    ensure!(component < count, "morph index out of range");
                    (
                        gltf::animation::Property::MorphTargetWeights,
                        Some(component),
                        1,
                    )
                }
                _ => anyhow::bail!("track {}: unsupported imported-node property", path()),
            };
            ensure!(
                matches!(&track.channel,Channel::Numeric(curve) if curve.width==width),
                "track {}: imported-node track shape mismatch",
                path()
            );
            track.asset_targets.push(AssetTarget {
                asset,
                node: imported.index(),
                property,
                component,
            });
            any = true;
        }
        Ok(any)
    }
    pub fn validate_asset_bindings(&self) -> Result<()> {
        for track in self.layers.iter().flat_map(|l| &l.tracks) {
            if let Some(v2::TrackTarget::Imported { asset, node, .. }) = &track.target {
                ensure!(
                    !track.asset_targets.is_empty() || track.asset_visibility.is_some(),
                    "unbound imported-node track [\"nodes\",{asset:?},\"nodes\",{node:?}]"
                );
            }
        }
        Ok(())
    }
    /// Content identity includes decoded buffer-backed keys, not their file names.
    pub fn input_identity(&self) -> blake3::Hash {
        let mut hash = blake3::Hasher::new();
        for layer in &self.layers {
            let relevant = |track: &&PropertyTrack| track.weight_target.is_some()
                || track.material_targets.as_ref().is_some_and(|targets| !targets.is_empty());
            if !layer.tracks.iter().any(|track| relevant(&track)) { continue; }
            for value in [layer.source.weight, layer.source.offset, layer.source.speed] { hash.update(&value.to_bits().to_le_bytes()); }
            hash.update(&[u8::from(layer.source.looping), u8::from(layer.source.blend == v2::Blend::Add)]);
            if layer.source.looping {
                if let Some((a,b)) = layer.span { hash.update(&a.to_bits().to_le_bytes()); hash.update(&b.to_bits().to_le_bytes()); }
            }
            for track in layer.tracks.iter().filter(relevant) {
                hash.update(track.target.as_ref().map(ToString::to_string).unwrap_or_default().as_bytes());
                match &track.channel {
                    Channel::Numeric(curve) => {
                        hash.update(&[match curve.interpolation {Interpolation::Step=>0,Interpolation::Linear=>1,Interpolation::Cubic=>2}]);
                        hash.update(bytemuck::cast_slice(&curve.times));
                        hash.update(bytemuck::cast_slice(&curve.values));
                    },
                    Channel::Discrete { times, values } => {
                        hash.update(bytemuck::cast_slice(times));
                        hash.update(format!("{values:?}").as_bytes());
                    },
                }
            }
        }
        for binding in &self.material_bindings {
            hash.update(format!("{}:{:?}:{:?}", binding.slot, binding.storage, binding.default).as_bytes());
        }
        hash.finalize()
    }
    pub fn targets(&self) -> impl Iterator<Item = &v2::TrackTarget> {
        self.layers
            .iter()
            .flat_map(|l| l.tracks.iter().filter_map(|t| t.target.as_ref()))
    }
    /// All names a camera cut can select, after decoding inline or buffered keys.
    pub fn camera_names(&self) -> impl Iterator<Item = &str> {
        self.layers.iter().flat_map(|layer| &layer.tracks).filter_map(|track| {
            if matches!(track.target, Some(v2::TrackTarget::Render(v2::RenderProperty::Camera))) {
                if let Channel::Discrete { values, .. } = &track.channel { return Some(values); }
            }
            None
        }).flatten().filter_map(|value| match value {
            Sample::String(name) => Some(name.as_ref()), _ => None,
        })
    }
    /// Whether any track writes material records at frame time.
    pub fn has_material_bindings(&self) -> bool {
        !self.material_bindings.is_empty()
    }
    /// The (material, input) pairs material tracks write.
    pub fn material_inputs(&self) -> Vec<(String, String)> {
        let mut out: Vec<_> = self
            .targets()
            .filter_map(|t| match t {
                v2::TrackTarget::Material { material, input } => {
                    Some((material.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
    pub fn bind_material(
        &mut self,
        (material, input): &(String, String),
        bindings: Vec<MaterialBinding>,
    ) -> Result<()> {
        let mut targets = Vec::new();
        for binding in bindings {
            let target = if let Some(index) = self
                .material_bindings
                .iter()
                .position(|b| b.slot == binding.slot && b.parameter == binding.parameter && b.storage == binding.storage)
            {
                index
            } else {
                let index = self.material_bindings.len();
                if binding.offset != 0. && !self.material_offsets.contains(&binding.offset) {
                    self.material_offsets.push(binding.offset);
                }
                self.material_bindings.push(binding);
                index
            };
            targets.push(target);
        }
        for track in self
            .layers
            .iter_mut()
            .flat_map(|l| &mut l.tracks)
            .filter(|t| {
                matches!(&t.target, Some(v2::TrackTarget::Material { material: m, input: i })
                    if m == material && i == input)
            })
        {
            for &target in &targets {
                let default = &self.material_bindings[target].default;
                ensure!(
                    match (&track.channel, default) {
                        (Channel::Numeric(curve), Sample::Numeric(v)) => curve.width == v.len(),
                        (Channel::Discrete { values, .. }, Sample::String(_)) =>
                            values.iter().all(|v| matches!(v, Sample::String(token) if self.material_bindings[target].enum_tokens.as_ref().is_some_and(|tokens| tokens.iter().any(|t| t == token.as_ref())))),
                        (Channel::Discrete { values, .. }, Sample::Bool(_)) =>
                            matches!(values[0], Sample::Bool(_)),
                        _ => false,
                    },
                    "material parameter type mismatch: [\"materials\",{material:?},\"inputs\",{input:?}]"
                );
            }
            track.material_targets.get_or_insert_with(Vec::new).extend(targets.iter().copied());
        }
        Ok(())
    }
    /// Record writes of the bindings at instance time `offset` (0: unshifted records).
    pub fn material_values<'a>(
        &'a self,
        samples: &'a Samples,
        offset: f32,
    ) -> impl Iterator<Item = (&'a MaterialBinding, &'a Sample)> {
        self.material_bindings
            .iter()
            .zip(&samples.material_values)
            .filter(move |(binding, _)| binding.offset == offset)
            .map(|(binding, value)| (binding, value))
    }
    /// Distinct nonzero instance time offsets with their own material record copies.
    pub fn material_offsets(&self) -> &[f32] {
        &self.material_offsets
    }
    /// Sample only the material tracks that write record copies at `offset`.
    pub fn sample_material_offset(&self, time: f32, offset: f32, samples: &mut Samples) {
        self.sample_into_where(time + offset, samples, |track| {
            track
                .material_targets
                .iter()
                .flatten()
                .any(|&t| self.material_bindings[t].offset == offset)
        });
    }
    pub fn samples(&self) -> Samples {
        Samples {
            values: self
                .layers
                .iter()
                .flat_map(|l| &l.tracks)
                .map(|track| match &track.channel {
                    Channel::Numeric(curve) => Sample::Numeric(vec![0.; curve.width]),
                    Channel::Discrete { values, .. } => values[0].clone(),
                })
                .collect(),
            weights: vec![0.; self.layers.iter().map(|l| l.tracks.len()).sum()],
            layer_weights: self.layers.iter().map(|l| l.source.weight).collect(),
            render: None,
            material_values: self
                .material_bindings
                .iter()
                .map(|b| b.default.clone())
                .collect(),
        }
    }
    pub fn sample_into(&self, time: f32, samples: &mut Samples) {
        self.sample_into_where(time, samples, |_| true);
    }
    pub fn sample_nodes_into(
        &self,
        time: f32,
        samples: &mut Samples,
        nodes: &[usize],
        assets: &[usize],
    ) {
        self.sample_into_where(time, samples, |track| {
            track
                .node_targets
                .iter()
                .any(|binding| nodes.binary_search(&binding.node).is_ok())
                || track.asset_visibility.is_some_and(|(asset,_)| assets.binary_search(&asset).is_ok())
                || track
                    .asset_targets
                    .iter()
                    .any(|target| assets.binary_search(&target.asset).is_ok())
        });
    }
    fn sample_into_where(
        &self,
        time: f32,
        samples: &mut Samples,
        wanted: impl Fn(&PropertyTrack) -> bool,
    ) {
        for (value, binding) in samples
            .material_values
            .iter_mut()
            .zip(&self.material_bindings)
        {
            value.clone_from(&binding.default);
        }
        for (weight, layer) in samples.layer_weights.iter_mut().zip(&self.layers) {
            *weight = layer.source.weight;
        }
        let mut index = 0;
        for (layer_index, layer) in self.layers.iter().enumerate() {
            let weight = samples.layer_weights[layer_index];
            let mut local = layer.source.offset + time * layer.source.speed;
            if layer.source.looping {
                if let Some((a, b)) = layer.span {
                    if b > a {
                        local = a + (local - a).rem_euclid(b - a);
                    }
                }
            }
            let additive = matches!(layer.source.blend, v2::Blend::Add);
            for track in &layer.tracks {
                let weight = if track.weight_target.is_some() || wanted(track) {
                    weight
                } else {
                    0.
                };
                samples.weights[index] = weight;
                if weight != 0. {
                    match &track.channel {
                        Channel::Numeric(curve) => {
                            let Sample::Numeric(value) = &mut samples.values[index] else {
                                unreachable!()
                            };
                            curve.sample(local, value);
                        }
                        Channel::Discrete { times, values } => {
                            samples.values[index] = values
                                [times.partition_point(|t| *t <= local).saturating_sub(1)]
                            .clone()
                        }
                    }
                    for &target in track.material_targets.iter().flatten() {
                        blend_sample(
                            &mut samples.material_values[target],
                            &samples.values[index],
                            weight,
                            additive,
                        );
                    }
                    if let Some(target) = track.weight_target {
                        if let Sample::Numeric(value) = &samples.values[index] {
                            blend(
                                std::slice::from_mut(&mut samples.layer_weights[target]),
                                value,
                                weight,
                                additive,
                                Combine::Add,
                            );
                        }
                    }
                }
                index += 1;
            }
        }
    }
    pub fn apply_asset_visibility(&self, asset: usize, samples: &Samples, visibility: &mut [v2::Visibility]) {
        self.apply_filtered(samples, |track| track.asset_visibility.is_some_and(|(a,_)| a == asset),
            |track, value, weight, _| {
                if weight >= 0.5 {
                    let Sample::Bool(value) = value else { unreachable!() };
                    visibility[track.asset_visibility.unwrap().1].visible = *value;
                }
            });
    }
    pub fn apply_asset(
        &self,
        asset: usize,
        samples: &Samples,
        mut apply: impl FnMut(AssetTarget, &Sample, f32, bool),
    ) {
        let mut index = 0;
        for layer in &self.layers {
            let additive = matches!(layer.source.blend, v2::Blend::Add);
            for track in &layer.tracks {
                for &target in &track.asset_targets {
                    if target.asset == asset && samples.weights[index] != 0. {
                        apply(
                            target,
                            &samples.values[index],
                            samples.weights[index],
                            additive,
                        );
                    }
                }
                index += 1;
            }
        }
    }
    pub fn apply_nodes(&self, samples: &Samples, values: &mut [Option<super::super::EvalNode>]) {
        self.apply_nodes_where(samples, values, |_| true);
    }
    pub fn apply_nodes_subset(
        &self,
        samples: &Samples,
        values: &mut [Option<super::super::EvalNode>],
        nodes: &[usize],
    ) {
        self.apply_nodes_where(samples, values, |node| nodes.binary_search(&node).is_ok());
    }
    fn apply_nodes_where(
        &self,
        samples: &Samples,
        values: &mut [Option<super::super::EvalNode>],
        wanted: impl Fn(usize) -> bool,
    ) {
        for binding in self
            .reset_bindings
            .iter()
            .filter(|binding| wanted(binding.node))
        {
            reset_node(&mut values[binding.node].as_mut().unwrap().node, binding);
        }
        let mut index = 0;
        for layer in &self.layers {
            let additive = matches!(layer.source.blend, v2::Blend::Add);
            for track in &layer.tracks {
                if samples.weights[index] != 0. {
                    for binding in track
                        .node_targets
                        .iter()
                        .filter(|binding| wanted(binding.node))
                    {
                        apply_node(
                            &mut values[binding.node].as_mut().unwrap().node,
                            &binding.property,
                            &samples.values[index],
                            samples.weights[index],
                            additive,
                        );
                    }
                }
                index += 1;
            }
        }
    }
    pub fn start_camera(&self, base: &v2::Render, time: f32) -> Option<String> {
        let mut samples = self.samples();
        self.sample_into(time, &mut samples);
        self.apply_render(&mut samples, base);
        samples.render.and_then(|render| render.camera)
    }
    /// Image clocks use the same evaluated layers, weights and loop mapping as poses.
    pub fn image_time(&self, samples: &Samples, name: &str, base: f32) -> f32 {
        let mut time = base;
        let mut index = 0;
        for layer in &self.layers {
            for track in &layer.tracks {
                if matches!(&track.target, Some(v2::TrackTarget::ImageTime { image }) if image == name) {
                    if let Sample::Numeric(value) = &samples.values[index] {
                        blend(std::slice::from_mut(&mut time), value, samples.weights[index],
                            matches!(layer.source.blend, v2::Blend::Add), Combine::Add);
                    }
                }
                index += 1;
            }
        }
        time
    }
    pub fn apply_render(&self, samples: &mut Samples, base: &v2::Render) {
        let render = samples.render.get_or_insert_with(|| base.clone());
        render.clone_from(base);
        let mut index = 0;
        for layer in &self.layers {
            let additive = matches!(layer.source.blend, v2::Blend::Add);
            for track in &layer.tracks {
                let weight = samples.weights[index];
                let property = match &track.target {
                    Some(v2::TrackTarget::Render(property)) => Some(*property),
                    _ => None,
                };
                if let (Some(property), true) = (property, weight != 0.) {
                    use v2::RenderProperty as R;
                    match (property, &samples.values[index]) {
                        (R::Background, Sample::Numeric(value)) => {
                            if let v2::Background::Color(color) = &mut render.background {
                                blend(color, value, weight, additive, Combine::Add);
                            }
                        },
                        (R::Camera, Sample::String(value)) if weight >= 0.5 => {
                            if let Some(camera) = &mut render.camera {
                                camera.clear();
                                camera.push_str(value);
                            } else {
                                render.camera = Some(value.to_string());
                            }
                        }
                        (R::MotionBlurShutter, Sample::Numeric(value)) => {
                            if let Some(m) = &mut render.motion_blur {
                                blend(std::slice::from_mut(&mut m.shutter), value, weight, additive, Combine::Add);
                            }
                        }
                        _ => {}
                    }
                }
                index += 1;
            }
        }
    }
    fn apply_filtered(
        &self,
        samples: &Samples,
        wanted: impl Fn(&PropertyTrack) -> bool,
        mut apply: impl FnMut(&PropertyTrack, &Sample, f32, bool),
    ) {
        let mut index = 0;
        for layer in &self.layers {
            let additive = matches!(layer.source.blend, v2::Blend::Add);
            for track in &layer.tracks {
                if track.weight_target.is_none() && samples.weights[index] != 0. && wanted(track) {
                    apply(
                        track,
                        &samples.values[index],
                        samples.weights[index],
                        additive,
                    );
                }
                index += 1;
            }
        }
    }
}
pub(super) fn numeric_buffer(
    scene: &v2::Scene,
    name: &str,
    base: &std::path::Path,
    workspace: Option<&std::collections::BTreeMap<String, crate::buffers::AttributeData>>,
) -> Result<(Vec<f32>, usize)> {
    let decoded;
    let data = if let Some(data) = workspace.and_then(|data| data.get(name)) {
        data
    } else {
        decoded = crate::buffers::AttributeData::decode(
            scene
                .buffers
                .get(name)
                .with_context(|| format!("unknown animation buffer {name}"))?,
            base,
        )?;
        &decoded
    };
    let values = data
        .words
        .iter()
        .map(|v| match data.element_type {
            v2::BufferType::I32 => (*v as i32) as f32,
            v2::BufferType::U32 | v2::BufferType::Uvec4 => *v as f32,
            _ => f32::from_bits(*v),
        })
        .collect();
    Ok((values, data.width))
}

/// Bind paths once; frame sampling touches only numeric values on installed nodes.
#[derive(Clone)]
struct NodeBinding {
    node: usize,
    property: v2::NodeProperty,
    base: Sample,
}
impl Plan {
    /// Whether a track animates this node's own translation, rotation or scale.
    pub fn transforms_node(&self, node: usize) -> bool {
        self.reset_bindings.iter().any(|b| b.node == node
            && matches!(b.property, v2::NodeProperty::Translation | v2::NodeProperty::Rotation | v2::NodeProperty::Scale))
    }
    pub fn mark_nodes(&mut self, nodes: &mut [super::super::EvalNode], default_camera: usize) -> Result<()> {
        self.reset_bindings.clear();
        self.cameras = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                matches!(n.node.kind, v2::NodeKind::Camera { .. }).then(|| (n.node.name.clone(), i))
            })
            .collect();
        let by_name: std::collections::HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.node.name.is_empty())
            .map(|(i, n)| (n.node.name.clone(), i))
            .collect();
        for layer in &mut self.layers {
            for track in &mut layer.tracks {
                if track.weight_target.is_some() {
                    continue;
                }
                let path = || {
                    track
                        .target
                        .as_ref()
                        .map_or("glTF clip channel".into(), |t| format!("track {t}"))
                };
                let mut binding = match &track.target {
                    None | Some(v2::TrackTarget::Imported { .. }) => {
                        ensure!(!track.asset_targets.is_empty() || track.asset_visibility.is_some(), "unbound {}", path());
                        continue;
                    }
                    Some(v2::TrackTarget::Material { .. }) => {
                        ensure!(
                            track.material_targets.is_some(),
                            "material track requires program parameter binding: {}",
                            path()
                        );
                        continue;
                    }
                    Some(v2::TrackTarget::LayerWeight { .. } | v2::TrackTarget::ImageTime { .. }) => continue,
                    Some(v2::TrackTarget::Render(property)) => {
                        use v2::RenderProperty as R;
                        if *property == R::Camera {
                            if let Channel::Discrete { values, .. } = &track.channel {
                                for value in values {
                                    ensure!(
                                        matches!(value,Sample::String(name) if self.cameras.contains_key(name.as_ref())),
                                        "camera track must reference installed cameras"
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    Some(v2::TrackTarget::Node { node, property }) => NodeBinding {
                        node: *by_name
                            .get(node)
                            .with_context(|| format!("unknown track node {node}"))?,
                        property: property.clone(),
                        base: Sample::Numeric(vec![]),
                    },
                };
                let node = &mut nodes[binding.node];
                if let Channel::Numeric(curve) = &track.channel {
                    let mut base = Vec::new();
                    ensure!(
                        node_numeric(&mut node.node, &binding.property, |value, _| base
                            .extend_from_slice(value))
                            && base.len() == curve.width,
                        "unsupported property or value shape: {}",
                        path()
                    );
                    binding.base = Sample::Numeric(base);
                } else {
                    let v2::NodeProperty::Visibility(ray) = binding.property else {
                        anyhow::bail!("{} requires numeric values", path());
                    };
                    ensure!(
                        matches!(&track.channel,Channel::Discrete{values,..} if matches!(values[0],Sample::Bool(_))),
                        "{} requires boolean values",
                        path()
                    );
                    binding.base = Sample::Bool(*node_bool(&mut node.node, ray));
                }
                node.dynamic = true;
                if !self
                    .reset_bindings
                    .iter()
                    .any(|b| b.node == binding.node && b.property == binding.property)
                {
                    self.reset_bindings.push(binding.clone());
                }
                track.node_targets = vec![binding];
            }
        }
        for i in 0..nodes.len() {
            nodes[i].dynamic |= nodes[i].parent.is_some_and(|p| nodes[p].dynamic);
        }

        self.compile_camera_events(default_camera);
        Ok(())
    }
    /// Half-open shot interval containing the nominal frame time.
    pub fn camera_bounds(&self, time: f32) -> (f32, f32) {
        let next = self.camera_events.partition_point(|event| event.0 <= time);
        let lower = next
            .checked_sub(1)
            .map_or(0., |index| self.camera_events[index].0);
        let upper = self
            .camera_events
            .get(next)
            .map_or(f32::INFINITY, |event| event.0.next_down());
        (lower, upper)
    }
    fn compile_camera_events(&mut self, fallback: usize) {
        let mut events = vec![0., self.duration];
        for layer in &self.layers {
            let speed = layer.source.speed;
            if speed == 0. {
                continue;
            }
            let period = layer
                .span
                .filter(|(a, b)| layer.source.looping && b > a)
                .map(|(a, b)| (b - a) / speed.abs());
            for track in &layer.tracks {
                if track.target != Some(v2::TrackTarget::Render(v2::RenderProperty::Camera))
                    && track.weight_target.is_none()
                {
                    continue;
                }
                let keys = match &track.channel {
                    Channel::Numeric(curve) => &curve.times,
                    Channel::Discrete { times, .. } => times,
                };
                for &key in keys
                    .iter()
                    .chain(layer.span.iter().flat_map(|(a, b)| [a, b]))
                {
                    let event = (key - layer.source.offset) / speed;
                    if let Some(period) = period {
                        let first = (-event / period).ceil() as i64;
                        let last = ((self.duration - event) / period).floor() as i64;
                        for cycle in first..=last {
                            let time = event + cycle as f32 * period;
                            if time >= 0. && time <= self.duration {
                                events.push(time);
                            }
                        }
                    } else if event >= 0. && event <= self.duration {
                        events.push(event);
                    }
                }
            }
        }
        events.sort_by(f32::total_cmp);
        events.dedup();
        let mut thresholds = vec![];
        for pair in events.windows(2) {
            let mut weights: Vec<_> = self
                .layers
                .iter()
                .map(|layer| vec![layer.source.weight as f64])
                .collect();
            for (index, layer) in self.layers.iter().enumerate() {
                let weight = weights[index].clone();
                if layer.tracks.iter().any(|track| {
                    track.target == Some(v2::TrackTarget::Render(v2::RenderProperty::Camera))
                }) {
                    let mut threshold = weight.clone();
                    threshold[0] -= 0.5;
                    thresholds.extend(
                        polynomial_roots(&threshold)
                            .into_iter()
                            .map(|u| pair[0] + (pair[1] - pair[0]) * u as f32),
                    );
                }
                for track in &layer.tracks {
                    if let (Some(target), Channel::Numeric(curve)) =
                        (track.weight_target, &track.channel)
                    {
                        let sample = weight_polynomial(curve, layer, pair[0], pair[1]);
                        let contribution = polynomial_multiply(&weight, &sample);
                        weights[target] = if matches!(layer.source.blend, v2::Blend::Add) {
                            polynomial_add(&weights[target], &contribution)
                        } else {
                            let inverse: Vec<_> = weight
                                .iter()
                                .enumerate()
                                .map(|(i, w)| if i == 0 { 1. - w } else { -w })
                                .collect();
                            polynomial_add(
                                &polynomial_multiply(&weights[target], &inverse),
                                &contribution,
                            )
                        };
                    }
                }
            }
        }
        events.extend(thresholds);
        events.sort_by(f32::total_cmp);
        events.dedup();
        let mut probes = vec![0.];
        for pair in events.windows(2) {
            probes.push(pair[0] + (pair[1] - pair[0]) * 0.5);
        }
        for event in events {
            probes.extend([
                event.next_down().max(0.),
                event,
                event.next_up().min(self.duration),
            ]);
        }
        probes.sort_by(f32::total_cmp);
        probes.dedup();
        let mut scratch = self.samples();
        let mut camera_at = |time| {
            self.sample_into(time, &mut scratch);
            self.camera_sampled(&scratch, fallback)
        };
        let mut cuts = vec![];
        for pair in probes.windows(2) {
            let before = camera_at(pair[0]);
            if camera_at(pair[1]) == before {
                continue;
            }
            let (mut lower, mut upper) = (pair[0].to_bits(), pair[1].to_bits());
            while upper - lower > 1 {
                let middle = lower + (upper - lower) / 2;
                if camera_at(f32::from_bits(middle)) == before {
                    lower = middle;
                } else {
                    upper = middle;
                }
            }
            let time = f32::from_bits(upper);
            cuts.push((time, camera_at(time)));
        }
        self.camera_start = Some(camera_at(0.));
        cuts.dedup();
        self.camera_events = cuts;
    }
    pub fn camera_sampled(&self, samples: &Samples, fallback: usize) -> usize {
        let mut camera = fallback;
        self.apply_filtered(
            samples,
            |track| track.target == Some(v2::TrackTarget::Render(v2::RenderProperty::Camera)),
            |_, value, weight, additive| {
                if weight >= 0.5 && !additive {
                    if let Sample::String(name) = value {
                        camera = *self.cameras.get(name.as_ref()).expect("validated camera track references an installed camera");
                    }
                }
            },
        );
        camera
    }
    pub fn camera_at(&self, time: f32, fallback: usize) -> usize {
        let upper = self.camera_events.partition_point(|event| event.0 <= time);
        upper
            .checked_sub(1)
            .map_or(self.camera_start.unwrap_or(fallback), |index| {
                self.camera_events[index].1
            })
    }
}
// Install-time scalar layer weights are piecewise polynomials. Isolating their
// threshold roots preserves camera cuts even when lower layers blend weights.
fn polynomial_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0.) + b.get(i).unwrap_or(&0.))
        .collect()
}
fn polynomial_multiply(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}
fn polynomial_value(p: &[f64], x: f64) -> f64 {
    p.iter().rev().fold(0., |v, c| v * x + c)
}
fn polynomial_roots(p: &[f64]) -> Vec<f64> {
    let mut p = p.to_vec();
    while p.len() > 1 && p.last() == Some(&0.) {
        p.pop();
    }
    if p.len() <= 1 {
        return vec![];
    }
    let derivative: Vec<_> = p
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, c)| i as f64 * c)
        .collect();
    let mut intervals = vec![0.];
    intervals.extend(polynomial_roots(&derivative));
    intervals.push(1.);
    let mut roots = vec![];
    for pair in intervals.windows(2) {
        let (mut a, mut b) = (pair[0], pair[1]);
        let mut fa = polynomial_value(&p, a);
        let fb = polynomial_value(&p, b);
        if fa == 0. {
            roots.push(a);
        }
        if fb == 0. {
            roots.push(b);
        }
        if fa.is_sign_negative() == fb.is_sign_negative() {
            continue;
        }
        loop {
            let mid = (a + b) * 0.5;
            if mid == a || mid == b {
                break;
            }
            let fm = polynomial_value(&p, mid);
            if fm == 0. {
                a = mid;
                b = mid;
                break;
            }
            if fm.is_sign_negative() == fa.is_sign_negative() {
                a = mid;
                fa = fm;
            } else {
                b = mid;
            }
        }
        roots.push((a + b) * 0.5);
    }
    roots.sort_by(f64::total_cmp);
    roots.dedup();
    roots
}
fn weight_polynomial(curve: &Curve, layer: &CompiledLayer, start: f32, end: f32) -> Vec<f64> {
    let span = (end - start) as f64 * layer.source.speed as f64;
    let mut midpoint =
        layer.source.offset as f64 + (start as f64 + end as f64) * 0.5 * layer.source.speed as f64;
    if let Some((a, b)) = layer.span.filter(|(a, b)| layer.source.looping && b > a) {
        midpoint = a as f64 + (midpoint - a as f64).rem_euclid((b - a) as f64);
    }
    let cubic = curve.interpolation == Interpolation::Cubic;
    let get = |key: usize, part: usize| curve.values[key * if cubic { 3 } else { 1 } + part] as f64;
    let upper = curve.times.partition_point(|t| (*t as f64) <= midpoint);
    let a = upper.saturating_sub(1);
    let b = upper.min(curve.times.len() - 1);
    if a == b || curve.interpolation == Interpolation::Step {
        return vec![get(a, usize::from(cubic))];
    }
    let dt = (curve.times[b] - curve.times[a]) as f64;
    let x = vec![
        (midpoint - span * 0.5 - curve.times[a] as f64) / dt,
        span / dt,
    ];
    let coefficients = if cubic {
        let (v0, v1, m0, m1) = (get(a, 1), get(b, 1), dt * get(a, 2), dt * get(b, 0));
        vec![
            v0,
            m0,
            -3. * v0 + 3. * v1 - 2. * m0 - m1,
            2. * v0 - 2. * v1 + m0 + m1,
        ]
    } else {
        vec![get(a, 0), get(b, 0) - get(a, 0)]
    };
    coefficients.iter().rev().fold(vec![0.], |p, c| {
        polynomial_add(&polynomial_multiply(&p, &x), &[*c])
    })
}
fn blend_sample(base: &mut Sample, value: &Sample, weight: f32, additive: bool) {
    match (base, value) {
        (Sample::Numeric(base), Sample::Numeric(value)) => {
            blend(base, value, weight, additive, Combine::Add)
        }
        (Sample::Bool(base), Sample::Bool(value)) if weight >= 0.5 && !additive => *base = *value,
        (Sample::String(base), Sample::String(value)) if weight >= 0.5 && !additive => {
            base.clone_from(value)
        }
        _ => {}
    }
}
fn reset_node(node: &mut v2::Node, binding: &NodeBinding) {
    match (&binding.base, &binding.property) {
        (Sample::Numeric(base), property) => {
            node_numeric(node, property, |value, _| value.copy_from_slice(base));
        }
        (Sample::Bool(base), v2::NodeProperty::Visibility(ray)) => {
            *node_bool(node, *ray) = *base;
        }
        _ => {}
    }
}
fn apply_node(
    node: &mut v2::Node,
    property: &v2::NodeProperty,
    sample: &Sample,
    weight: f32,
    additive: bool,
) {
    match (sample, property) {
        (Sample::Numeric(sample), property) => {
            node_numeric(node, property, |base, kind| {
                blend(base, sample, weight, additive, kind)
            });
        }
        (Sample::Bool(sample), v2::NodeProperty::Visibility(ray)) if weight >= 0.5 && !additive => {
            *node_bool(node, *ray) = *sample;
        }
        _ => {}
    }
}
fn node_bool(node: &mut v2::Node, ray: v2::Ray) -> &mut bool {
    let v = &mut node.visibility;
    match ray {
        v2::Ray::All => &mut v.visible,
        v2::Ray::Camera => &mut v.camera,
        v2::Ray::Shadow => &mut v.shadow,
        v2::Ray::Diffuse => &mut v.diffuse,
        v2::Ray::Glossy => &mut v.glossy,
        v2::Ray::Transmission => &mut v.transmission,
    }
}
/// Apply `apply` to the numeric value of `property`; false when the node has none.
fn node_numeric(
    node: &mut v2::Node,
    property: &v2::NodeProperty,
    mut apply: impl FnMut(&mut [f32], Combine),
) -> bool {
    use v2::{NodeKind as K, NodeProperty as P};
    macro_rules! scalar {
        ($v:expr) => {{
            apply(std::slice::from_mut($v), Combine::Add);
            true
        }};
    }
    match (&mut node.kind, property) {
        (_, P::Translation) => {
            apply(&mut node.transform.translation, Combine::Add);
            true
        }
        (_, P::Rotation) => {
            apply(node.transform.rotation_mut(), Combine::Rotation);
            true
        }
        (_, P::Scale) => {
            apply(&mut node.transform.scale, Combine::Multiply);
            true
        }
        (K::Mesh { morph_weights, .. }, P::MorphWeight(name)) => {
            match morph_weights.get_mut(name) {
                Some(value) => scalar!(value),
                None => false,
            }
        }
        (
            K::Light {
                color,
                shape,
                image, material,
                ..
            },
            P::Color,
        ) => {
            apply(
                color.get_or_insert(shape.default_color(image.is_some() || material.is_some())),
                Combine::Add,
            );
            true
        }
        (
            K::Light {
                energy: Some(value),
                ..
            },
            P::Energy,
        ) => scalar!(value),
        (K::Light { temperature: Some(value), .. }, P::Temperature) => scalar!(value),
        (K::Light { angle, .. }, P::Angle) => scalar!(angle),
        (K::Light { radius, .. }, P::Radius) => scalar!(radius),
        (K::Light { spot_blend, .. }, P::SpotBlend) => scalar!(spot_blend),
        (K::Light { spot_size, .. }, P::SpotSize) => scalar!(spot_size),
        (K::Light { size: width, .. }, P::Width) => scalar!(width),
        (K::Light { size: width, size_y: height, .. }, P::Height) => {
            let height = height.get_or_insert(*width);
            scalar!(height)
        },
        _ => false,
    }
}

#[cfg(test)]
mod quaternion_tests {
    use super::*;
    #[test]
    fn antipodal_cubic_midpoint_retains_valid_direction() -> Result<()> {
        let track: v2::Track = vork_scene::v2::from_test_value(
            serde_json::json!({"path":["nodes","actor","rotation"],"keys":[[0,[0,0,0,1],[0,0,0,0],[0,0,0,0]],[1,[0,0,0,-1],[0,0,0,0],[0,0,0,0]]],"interpolation":"CUBICSPLINE"}),
        )?;
        let v2::Values::Inline(values) = &track.values else { unreachable!() };
        let curve = Curve::authored(&track, true, vec![0., 1.], values)?;
        let mut value = vec![];
        curve.sample(0.5, &mut value);
        assert_eq!(value, [0., 0., 0., 1.]);
        Ok(())
    }
}

#[cfg(test)]
mod camera_cut_tests {
    use super::*;
    #[test]
    fn node_radius_blends_point_and_spot_emitters() {
        for shape in ["point", "spot"] {
            let mut node: v2::Node = vork_scene::v2::from_test_value(serde_json::json!({"type":"light","shape":shape,"radius":0.1})).unwrap();
            apply_node(&mut node, &v2::NodeProperty::Radius, &Sample::Numeric(vec![0.3]), 0.5, false);
            let v2::NodeKind::Light { radius, .. } = node.kind else { unreachable!() };
            assert!((radius-0.2).abs()<1e-6);
        }
    }
    fn plan(animation: serde_json::Value) -> Result<Plan> {
        let input: v2::Scene = vork_scene::v2::from_test_value(serde_json::json!({
            "version":2,"render":{"duration":2,"camera":"a"},
            "nodes":[{"type":"camera","name":"a","material":{"program":"std/camera","inputs":{"projection":"persp"}}},{"type":"camera","name":"b","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
            "animation":animation
        }))?;
        input.validate()?;
        let mut plan = Plan::compile(&input, std::path::Path::new("."))?;
        let mut nodes: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|name| crate::EvalNode { camera: None,
                node: vork_scene::v2::from_test_value(serde_json::json!({"type":"camera","name":name}))
                    .unwrap(),
                parent: None,
                world: glam::Mat4::IDENTITY,
                dynamic: false,
                embedded: None,
            })
            .collect();
        plan.mark_nodes(&mut nodes, 0)?;
        Ok(plan)
    }
    #[test]
    fn start_camera_uses_negative_offset_loop_and_ordered_layers() -> Result<()> {
        let plan = plan(serde_json::json!([
            {"tracks":[{"path":["render","camera"],"keys":[[0,"a"]]}]},
            {"offset":-0.25,"loop":true,"tracks":[{
                "path":["render","camera"],"keys":[[0,"a"],[0.5,"b"],[1,"a"]]
            }]}
        ]))?;
        assert_eq!(plan.start_camera(&v2::Render::default(), 0.).as_deref(), Some("b"));
        assert_eq!(plan.start_camera(&v2::Render::default(), 0.5).as_deref(), Some("a"));
        assert_eq!(plan.camera_at(0., 0), 1);
        Ok(())
    }
    #[test]
    fn start_camera_uses_buffer_backed_layer_weight() -> Result<()> {
        let scene: v2::Scene = vork_scene::v2::from_test_value(serde_json::json!({
            "version":2,"nodes":[{"type":"camera","name":"a"},{"type":"camera","name":"b"}],
            "buffers":{"times":{"type":"f32","data":[0,1]},"weights":{"type":"f32","data":[1,0]}},
            "animation":[
                {"tracks":[{"path":["animation","cut","weight"],"keys":{"times":"times","values":"weights"}}]},
                {"name":"cut","weight":0,"tracks":[{"path":["render","camera"],"keys":[[0,"b"]]}]}
            ]
        }))?;
        let plan = Plan::compile(&scene, std::path::Path::new("."))?;
        assert_eq!(plan.start_camera(&scene.render, 0.).as_deref(), Some("b"));
        Ok(())
    }
    #[test]
    fn camera_cut_shutter_interval_respects_offset_speed_and_loop() -> Result<()> {
        let plan = plan(
            serde_json::json!([{"offset":0.2,"speed":2,"loop":true,"tracks":[{
                "path":["render","camera"],"interpolation":"STEP","keys":[[0,"a"],[0.4,"b"],[1,"a"]]
            }]}]),
        )?;
        let (lower, upper) = plan.camera_bounds(0.25);
        assert!((lower - 0.1).abs() < 1e-6);
        assert!((upper - 0.4).abs() < 1e-6);
        assert_eq!(plan.camera_at(upper, 0), 1);
        assert_eq!(plan.camera_at(upper.next_up(), 0), 0);
        let (lower, upper) = plan.camera_bounds(0.75);
        assert!((lower - 0.6).abs() < 1e-6);
        assert!((upper - 0.9).abs() < 1e-6);
        Ok(())
    }
    #[test]
    fn cubic_layer_weight_keeps_both_camera_threshold_crossings() -> Result<()> {
        let plan = plan(serde_json::json!([
            {"tracks":[{"path":["animation","cut","weight"],"keys":[[0,0,0,4],[1,0,-4,0]],"interpolation":"CUBICSPLINE"}]},
            {"name":"cut","weight":0,"tracks":[{"path":["render","camera"],"keys":[[0,"b"]],"interpolation":"STEP"}]}
        ]))?;
        let (lower, upper) = plan.camera_bounds(0.5);
        assert!((lower - (1. - 0.5_f32.sqrt()) * 0.5).abs() < 1e-5);
        assert!((upper - (1. + 0.5_f32.sqrt()) * 0.5).abs() < 1e-5);
        assert_eq!(plan.camera_at(lower.next_down(), 0), 0);
        assert_eq!(plan.camera_at(lower, 0), 1);
        assert_eq!(plan.camera_at(upper, 0), 1);
        assert_eq!(plan.camera_at(upper.next_up(), 0), 0);
        Ok(())
    }
    #[test]
    fn visible_defaults_to_step_and_rejects_continuous_or_additive_blends() -> Result<()> {
        plan(serde_json::json!([{"tracks":[{
            "path":["nodes","a","visible"],"keys":[[0,false],[1,true]]
        }]}]))?;
        for interpolation in ["LINEAR", "CUBICSPLINE"] {
            assert!(plan(serde_json::json!([{"tracks":[{
                "path":["nodes","a","visible"],"keys":[[0,false],[1,true]],"interpolation":interpolation
            }]}])).is_err());
        }
        assert!(plan(serde_json::json!([{"blend":"add","tracks":[{
            "path":["nodes","a","visible"],"keys":[[0,false],[1,true]]
        }]}])).is_err());
        Ok(())
    }
    #[test]
    fn additive_boolean_tracks_fail_validation() {
        assert!(plan(serde_json::json!([{"blend":"add","tracks":[{
            "path":["nodes","a","visible_camera"],"keys":[[0,true],[1,false]],"interpolation":"STEP"
        }]}])).is_err());
    }
}

/// Conservative component envelopes, including cubic Bezier control hulls and
/// ordered layer weights. Used only while installing projected curve detail.
impl Plan {
    pub(crate) fn node_envelope(
        &self,
        node: &str,
        property: &v2::NodeProperty,
        base: &[f32],
    ) -> Vec<[f32; 2]> {
        fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
            [a[0] + b[0], a[1] + b[1]]
        }
        fn mul(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
            let v = [a[0] * b[0], a[0] * b[1], a[1] * b[0], a[1] * b[1]];
            [
                v.into_iter().fold(f32::INFINITY, f32::min),
                v.into_iter().fold(f32::NEG_INFINITY, f32::max),
            ]
        }
        fn combine(a: [f32; 2], b: [f32; 2], w: [f32; 2], additive: bool, scale: bool) -> [f32; 2] {
            if additive && scale {
                // A signed power; retain both signs if the sample crosses zero.
                let magnitude = [b[0].abs(), b[1].abs()];
                let low = if b[0] <= 0. && b[1] >= 0. {
                    0.
                } else {
                    magnitude[0].min(magnitude[1])
                };
                let high = magnitude[0].max(magnitude[1]);
                let powers = [
                    low.powf(w[0]),
                    low.powf(w[1]),
                    high.powf(w[0]),
                    high.powf(w[1]),
                ];
                let lo = powers.into_iter().fold(f32::INFINITY, f32::min);
                let hi = powers.into_iter().fold(f32::NEG_INFINITY, f32::max);
                let factor = if b[0] >= 0. {
                    [lo, hi]
                } else if b[1] <= 0. {
                    [-hi, -lo]
                } else {
                    [-hi, hi]
                };
                return mul(a, factor);
            }
            if additive {
                return add(a, mul(w, b));
            }
            if w == [1.; 2] {
                return b;
            }
            if w[0] >= 0. && w[1] <= 1. {
                return [a[0].min(b[0]), a[1].max(b[1])];
            }
            add(mul([1. - w[1], 1. - w[0]], a), mul(w, b))
        }
        fn hull(curve: &Curve) -> Vec<[f32; 2]> {
            // Euler storage holds three angles; sampled values are unit quaternions.
            if curve.euler {
                return vec![[-1., 1.]; 4];
            }
            let mut result = vec![[f32::INFINITY, f32::NEG_INFINITY]; curve.width];
            let cubic = curve.interpolation == Interpolation::Cubic;
            for key in 0..curve.times.len() {
                for (component, bound) in result.iter_mut().enumerate() {
                    let at = |key: usize, part: usize| {
                        curve.values
                            [(key * if cubic { 3 } else { 1 } + part) * curve.width + component]
                    };
                    let value = at(key, usize::from(cubic));
                    let mut include = |v: f32| {
                        bound[0] = bound[0].min(v);
                        bound[1] = bound[1].max(v);
                    };
                    include(value);
                    if cubic && key + 1 < curve.times.len() {
                        let dt = (curve.times[key + 1] - curve.times[key]) / 3.;
                        include(value + dt * at(key, 2));
                        include(at(key + 1, 1) - dt * at(key + 1, 0));
                    }
                }
            }
            result
        }
        let mut result: Vec<_> = base.iter().map(|v| [*v; 2]).collect();
        let mut weights: Vec<_> = self.layers.iter().map(|l| [l.source.weight; 2]).collect();
        for (index, layer) in self.layers.iter().enumerate() {
            let weight = weights[index];
            for track in &layer.tracks {
                let matches_node = matches!(&track.target, Some(v2::TrackTarget::Node { node: n, property: p }) if n == node && p == property);
                if track.weight_target.is_none() && !matches_node {
                    continue;
                }
                let Channel::Numeric(curve) = &track.channel else {
                    continue;
                };
                let bound = hull(curve);
                let additive = matches!(layer.source.blend, v2::Blend::Add);
                if let Some(target) = track.weight_target {
                    weights[target] = combine(weights[target], bound[0], weight, additive, false);
                }
                if matches_node {
                    for (out, bound) in result.iter_mut().zip(bound) {
                        *out = combine(
                            *out,
                            bound,
                            weight,
                            additive,
                            *property == v2::NodeProperty::Scale,
                        );
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod envelope_tests {
    use super::*;
    #[test]
    fn cubic_envelope_covers_interior_overshoot_and_layer_combination() -> Result<()> {
        let scene: v2::Scene = vork_scene::v2::from_test_value(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","name":"camera"}],
            "animation":[{"tracks":[{"path":["nodes","camera","scale"],"keys":[[0,[35,35,35],[0,0,0],[50,50,50]],[2,[35,35,35],[-50,-50,-50],[0,0,0]]],"interpolation":"CUBICSPLINE"}]},{"weight":0.5,"blend":"add","tracks":[{"path":["nodes","camera","scale"],"keys":[[0,[2,2,2]],[2,[4,4,4]]]}]}]
        }))?;
        let plan = Plan::compile(&scene, std::path::Path::new("."))?;
        let bounds = plan.node_envelope("camera", &v2::NodeProperty::Scale, &[50.;3])[0];
        assert!(bounds[1] > 50.);
        for step in 0..=100 {
            let time = step as f32 / 50.;
            let mut value = vec![];
            let Channel::Numeric(curve) = &plan.layers[0].tracks[0].channel else {
                unreachable!()
            };
            curve.sample(time, &mut value);
            let actual = value[0] * (2. + time).powf(0.5);
            assert!(actual >= bounds[0] && actual <= bounds[1]);
        }
        Ok(())
    }
}

/// Bake time-invariant material tracks before any stage installs resources. Keep
/// layer spans so removing a constant endpoint cannot change another track's loop.
pub(crate) fn fold_material_inputs(scene: &mut v2::Scene, base: &std::path::Path) -> Result<()> {
    if !scene.animation.iter().any(|l| l.tracks.iter().flatten().any(|t| t.path.first().is_some_and(|p| p == "materials"))) { return Ok(()); }
    let mut plan = Plan::compile_with_buffers(scene, base, None)?;
    let constant = |channel: &Channel| match channel {
        Channel::Numeric(curve) => {
            let cubic = curve.interpolation == Interpolation::Cubic;
            let stride = curve.width * if cubic {3} else {1};
            let first = &curve.values[if cubic {curve.width} else {0}..if cubic {curve.width * 2} else {curve.width}];
            curve.values.chunks_exact(stride).all(|key| {
                if cubic { &key[curve.width..curve.width * 2] == first && key[..curve.width].iter().chain(&key[curve.width * 2..]).all(|v| *v == 0.) }
                else { key == first }
            })
        },
        Channel::Discrete { values, .. } => values.iter().all(|v| v == &values[0]),
    };
    // A changing layer weight can make even a constant key time-varying.
    if plan.layers.iter().flat_map(|l| &l.tracks).any(|t| t.weight_target.is_some() && !constant(&t.channel)) { return Ok(()); }
    let mut fields = std::collections::BTreeMap::<(String, String), bool>::new();
    for track in plan.layers.iter().flat_map(|l| &l.tracks) {
        if let Some(v2::TrackTarget::Material {material, input}) = &track.target {
            *fields.entry((material.clone(), input.clone())).or_insert(true) &= constant(&track.channel);
        }
    }
    let fields: Vec<_> = fields.into_iter().filter_map(|(field, fixed)| fixed.then_some(field)).collect();
    for (slot, (name, input)) in fields.iter().enumerate() {
        let value = scene.materials[name].inputs.get(input);
        let default = match value {
            Some(v2::Input::Number(v)) => vec![*v],
            Some(v2::Input::Vector(v)) => v.clone(),
            Some(v2::Input::Bool(_)) => continue,
            Some(v2::Input::Attribute(binding)) if scene.materials[name].program == "std/openpbr" => match &binding.factor {
                Some(v2::Factor::Number(v)) => vec![*v],
                Some(v2::Factor::Vector(v)) => v.clone(),
                None => vec![1.; if matches!(v2::OpenPbrParam::find(input).unwrap().kind, v2::OpenPbrKind::Color3 | v2::OpenPbrKind::Vector3) {3} else {1}],
            },
            Some(_) => continue,
            None if scene.materials[name].program == "std/openpbr" => {
                let p = v2::OpenPbrParam::find(input).unwrap();
                if p.kind == v2::OpenPbrKind::Vector3 {vec![0.;3]} else {p.default.to_vec()}
            },
            None => {
                let track = plan.layers.iter().flat_map(|l| &l.tracks).find(|t| matches!(&t.target, Some(v2::TrackTarget::Material {material, input: i}) if material == name && i == input)).unwrap();
                let Channel::Numeric(curve) = &track.channel else {continue};
                vec![0.; curve.width]
            }
        };
        plan.bind_material(&(name.clone(), input.clone()), vec![MaterialBinding {slot, parameter: 0, storage: None, enum_tokens: None, default: Sample::Numeric(default), offset: 0.}])?;
    }
    let mut samples = plan.samples();
    plan.sample_material_offset(0., 0., &mut samples);
    let mut folded = std::collections::BTreeSet::new();
    for (binding, value) in plan.material_values(&samples, 0.) {
        let (name, input) = &fields[binding.slot];
        let Sample::Numeric(value) = value else {continue};
        let value = if value.len() == 1 {v2::Input::Number(value[0])} else {v2::Input::Vector(value.clone())};
        let material = scene.materials.get_mut(name).unwrap();
        let factor = match &value {
            v2::Input::Number(v) => v2::Factor::Number(*v),
            v2::Input::Vector(v) => v2::Factor::Vector(v.clone()),
            _ => unreachable!(),
        };
        match material.inputs.get_mut(input) {
            Some(v2::Input::Attribute(binding)) => binding.factor = Some(factor),
            _ => { material.inputs.insert(input.clone(), value); }
        }
        folded.insert((name.clone(), input.clone()));
    }
    for (layer, compiled) in scene.animation.iter_mut().zip(&plan.layers) {
        layer.source_span = compiled.span;
        if let Some(tracks) = &mut layer.tracks {
            tracks.retain(|track| !matches!(track.path.as_slice(), [root, name, inputs, input] if root == "materials" && inputs == "inputs" && folded.contains(&(name.clone(), input.clone()))));
        }
    }
    let referenced: std::collections::BTreeSet<_> = scene.animation.iter().flat_map(|l| l.tracks.iter().flatten())
        .filter_map(|t| match t.path.as_slice() { [root, name, field] if root == "animation" && field == "weight" => Some(name.clone()), _ => None }).collect();
    scene.animation.retain(|layer| layer.clip.is_some() || layer.tracks.as_ref().is_some_and(|t| !t.is_empty()) || layer.name.as_ref().is_some_and(|name| referenced.contains(name)));
    Ok(())
}
