//! Per-view light lists shared by raster surfaces and media, including path fog.
use crate::*;

pub(crate) fn encode(encoder: &mut wgpu::CommandEncoder, f: &crate::view::ViewState) {
    let batch = &f.lighting;
    if let Some((pipeline, bind, grid)) = batch.clusters.as_ref() {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cluster light culling"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &f.queries,
                beginning_of_pass_write_index: Some(timing::query("clusters")),
                end_of_pass_write_index: Some(timing::query("clusters") + 1),
            }),
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        let group = constants::CLUSTER_GROUP;
        pass.dispatch_workgroups(grid[0].div_ceil(group), grid[1].div_ceil(group), grid[2]);
    }
}
