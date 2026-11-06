#ifndef VORK_COMPOSITE
#define VORK_COMPOSITE 0
#endif
// Christensen–Burley screen diffusion, independently for every visibility slot.
import vork_prelude;
import vork_constants;
import subsurface_diffusion;
import decoupled_slots;
#if VORK_COMPOSITE
[[vk::binding(0,0)]] Texture2D<float4> correction : register(t0);
[[vk::binding(1,0)]] Texture2D<float4> ambient_correction : register(t1);
[shader("vertex")]
float4 subsurface_vs(uint id : SV_VertexID) : SV_Position {
    return float4(float2(float((id<<1u)&2u),float(id&2u))*float2(2.,-2.)+float2(-1.,1.),0.,1.);
}
struct Correction {float4 color : SV_Target0;float4 ambient : SV_Target1;};
[shader("fragment")]
Correction subsurface_compose(float4 pos : SV_Position) {
    let p=int2(pos.xy);
    return Correction(correction.Load(int3(p,0)),ambient_correction.Load(int3(p,0)));
}
#else
[[vk::binding(0,0)]] ConstantBuffer<Frame> frame : register(b0);
[[vk::binding(1,0)]] StructuredBuffer<uint> slots : register(t1);
[[vk::binding(2,0)]] StructuredBuffer<uint> records : register(t2);
[[vk::binding(3,0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> correction : register(u0);
[[vk::binding(4,0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> ambient_correction : register(u1);
[[vk::binding(5,0)]] ConstantBuffer<uint4> probe_range : register(b3);
[[vk::binding(6,0)]] RWStructuredBuffer<float> accumulation : register(u4);
[shader("compute")]
[numthreads(8,8,1)]
void subsurface_diffuse(uint3 tid : SV_DispatchThreadID) {
    uint w,h;correction.GetDimensions(w,h);let size=uint2(w,h);let p=int2(tid.xy);
    if (any(tid.xy>=size)) {return;}
    // The nominal/control view can use a shorter sequence than the colour view.
    let count=REG_SUBSURFACE_BASE_PROBES+frame.integrator.z;
    let end=min(probe_range.y,count);
    if (probe_range.x>=end) {return;}
    let layout=SlotLayout(w*h,uint(frame.sampling.w));let pixel=tid.y*w+tid.x;
    var color=float3(0.);var ambient=float3(0.);
    for (var at=layout.first(slots,pixel);at!=0xffffffffu;at=layout.next(slots,pixel,at)) {
        let base=layout.record(at)*REG_SUBSURFACE_ACCUMULATION_WORDS;
        var sum=float3(0.);var ambient_sum=float3(0.);var weights=float3(0.);
        if (probe_range.x!=0u) {
            sum=float3(accumulation[base],accumulation[base+1u],accumulation[base+2u]);
            ambient_sum=float3(accumulation[base+3u],accumulation[base+4u],accumulation[base+5u]);
            weights=float3(accumulation[base+6u],accumulation[base+7u],accumulation[base+8u]);
        }
        float3 c,a;diffuse_range(slots,records,layout,at,p,size,frame.projection.yx,frame.flags.x>.5,frame.integrator.z,uint(frame.temporal.x),probe_range.x,end,sum,ambient_sum,weights,c,a);
        if (end<count) {
            for (uint channel=0u;channel<3u;channel++) {
                accumulation[base+channel]=sum[channel];
                accumulation[base+3u+channel]=ambient_sum[channel];
                accumulation[base+6u+channel]=weights[channel];
            }
        } else {color+=c;ambient+=a;}
    }
    if (end==count) {
        correction[p]=float4(color,0.);ambient_correction[p]=float4(ambient,0.);
    }
}
#endif
