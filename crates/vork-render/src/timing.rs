//! Named diagnostic spans plus a separate internal scheduling interval.
use crate::constants;

const _: () = assert!(constants::GPU_SPANS.len() <= u64::BITS as usize);

pub(crate) fn count() -> u32 {
    sample_query() + 2
}

/// Internal counters never contribute to additive diagnostic span totals.
pub(crate) fn sample_query() -> u32 {
    constants::GPU_SPANS.len() as u32 * 2
}

pub(crate) fn boundary(r: &crate::Renderer, encoder: &mut wgpu::CommandEncoder, queries: &wgpu::QuerySet, index: u32) {
    if r.adapter_info.backend == wgpu::Backend::Metal {
        r.timestamp_marker.get_or_init(|| Marker::new(&r.device)).encode(encoder, queries, index);
    } else if r.device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        encoder.write_timestamp(queries, index);
    } else {
        drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sample scheduling boundary"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        }));
    }
}
pub(crate) fn query(name: &str) -> u32 {
    constants::GPU_SPANS
        .iter()
        .position(|&span| span == name)
        .expect("unregistered GPU timing span") as u32
        * 2
}

/// Each written pair has an aligned resolve destination; one exposure copy
/// transfers the retained results without per-span staging copies.
pub(crate) fn resolve_stride() -> u64 {
    u64::from(count() / 2 + 1) * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT
}
pub(crate) fn resolve_offset(sample: usize, query: u32) -> u64 {
    debug_assert_eq!(query % 2, 0);
    sample as u64 * resolve_stride() + u64::from(query / 2) * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT
}
/// A group can begin and end in different query sets. Each endpoint needs an
/// aligned destination; singleton groups resolve the ordinary adjacent pair.
pub(crate) fn group_end_offset(sample: usize) -> u64 {
    resolve_offset(sample, sample_query()) + wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT
}

pub(crate) fn mask(name: &str) -> u64 {
    1 << (query(name) / 2)
}

pub(crate) fn spans(mask: u64) -> impl Iterator<Item = &'static str> {
    constants::GPU_SPANS
        .iter()
        .copied()
        .enumerate()
        .filter_map(move |(index, name)| (mask & (1 << index) != 0).then_some(name))
}

/// Zero is an unwritten counter, not a zero-cost pass. Equal nonzero samples
/// are valid for work shorter than the timestamp counter's resolution.
pub(crate) fn elapsed_ms(begin: u64, end: u64, period: f32) -> Option<f64> {
    if begin == 0 {
        return None;
    }
    Some(end.checked_sub(begin)? as f64 * period as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_or_reversed_timestamps_are_not_free_work() {
        assert_eq!(super::elapsed_ms(0, 0, 1.), None);
        assert_eq!(super::elapsed_ms(0, 20, 1.), None);
        assert_eq!(super::elapsed_ms(20, 0, 1.), None);
        assert_eq!(super::elapsed_ms(20, 19, 1.), None);
        assert_eq!(super::elapsed_ms(20, 20, 1.), Some(0.));
        assert_eq!(super::elapsed_ms(20, 1_000_020, 2.), Some(2.));
    }
}

/// Metal does not reliably sample empty compute passes or standalone blits.
/// A single real store gives a boundary marker for work spanning several views.
/// Lazily shared per renderer; its scratch value has no rendering meaning.
#[derive(Clone)]
pub(crate) struct Marker {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
}
impl Marker {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader=shader_module(device,wgpu::ShaderModuleDescriptor {
            label: Some("timestamp boundary marker"),
            source: wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var<storage,read_write> marker:u32; @compute @workgroup_size(1) fn main() {marker=0u;}".into()),
        });
        let start=std::time::Instant::now();
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("timestamp boundary marker"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        creation(device,"timestamp boundary marker","pipeline",0.,start.elapsed().as_secs_f64()*1000.);
        let scratch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp marker scratch"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scratch.as_entire_binding(),
            }],
        });
        Self { pipeline, bind }
    }
    pub(crate) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queries: &wgpu::QuerySet,
        index: u32,
    ) {
        self.encode_timestamps(encoder, wgpu::ComputePassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: None,
        });
    }
    pub(crate) fn encode_timestamps(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: wgpu::ComputePassTimestampWrites<'_>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("timestamp boundary marker"),
            timestamp_writes: Some(timestamps),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

/// Process-local creation receipts. Concurrent durations overlap; retain every
/// call rather than collapsing repeated labels or mixing Slang with driver JIT.
#[derive(Clone, serde::Serialize)]
pub(crate) struct Creation {
    #[serde(skip)]
    device: u64,
    pub(crate) label: String,
    pub(crate) kind: &'static str,
    slang_ms: f64,
    driver_ms: f64,
}
static CREATIONS: std::sync::Mutex<Vec<Creation>> = std::sync::Mutex::new(Vec::new());
// Device ids are unique among live devices only because every Renderer opens
// its device from one process-wide wgpu instance (Renderer::create).
fn device_id(device: &wgpu::Device) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash=std::collections::hash_map::DefaultHasher::new();
    device.hash(&mut hash);
    hash.finish()
}
pub(crate) fn creation(device: &wgpu::Device, label: &str, kind: &'static str, slang_ms: f64, driver_ms: f64) {
    CREATIONS.lock().unwrap().push(Creation {device:device_id(device),label:label.into(),kind,slang_ms,driver_ms});
}
pub(crate) fn creations(device: &wgpu::Device) -> Vec<Creation> {
    let id=device_id(device);
    CREATIONS.lock().unwrap().iter().filter(|c|c.device==id).cloned().collect()
}
pub(crate) fn shader_module(device: &wgpu::Device, descriptor: wgpu::ShaderModuleDescriptor<'_>) -> wgpu::ShaderModule {
    let start=std::time::Instant::now();
    let label=descriptor.label.unwrap_or("shader module").to_owned();
    let module=device.create_shader_module(descriptor);
    creation(device,&label,"shader module",0.,start.elapsed().as_secs_f64()*1000.);
    module
}

pub(crate) fn forget(device: &wgpu::Device) {
    let id=device_id(device);
    CREATIONS.lock().unwrap().retain(|c|c.device!=id);
}
