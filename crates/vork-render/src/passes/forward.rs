//! The controls pass: depth-equal control targets (ambient, IDs, normals, motion).
use crate::*;

pub(crate) fn encode(encoder: &mut wgpu::CommandEncoder, f: &crate::view::ViewState) {
    let mut pass = begin_pass(encoder,f,"forward depth equal",wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),"forward");
    let bundles = f.bundles();
    // Colour comes from the decoupled shade pass (or the path tracer).
    pass.execute_bundles([&bundles.controls]);
}

pub(crate) fn begin_pass<'a>(encoder:&'a mut wgpu::CommandEncoder,f:&'a crate::view::ViewState,label:&'a str,load:wgpu::LoadOp<wgpu::Color>,span:&str) -> wgpu::RenderPass<'a> {
    let control=|view| wgpu::RenderPassColorAttachment {
        view, depth_slice:None, resolve_target:None,
        ops:wgpu::Operations {load,store:wgpu::StoreOp::Store},
    };
    let attachments=[Some(control(&f.ids)),Some(control(&f.normals)),
        Some(control(&f.ambient)),Some(control(&f.motion)),f.denoising_albedo.as_ref().map(control)];
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label:Some(label),
        color_attachments:&attachments[..pipelines::control_formats(f.denoising_albedo.is_some()).len()],
        depth_stencil_attachment:Some(depth_attachment(&f.depth,false,false)),
        timestamp_writes:Some(timestamps(&f.queries,timing::query(span))),
        occlusion_query_set:None,multiview_mask:None,
    })
}
