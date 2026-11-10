// Exact occupancy of the complete GI reconstruction and upsampling support.
// Seed once from the same weighted payloads as resolve. Subsequent passes
// dilate contiguous integer intervals, independently along each image axis.
import vork_constants;
import vork_prelude;
[[vk::binding(0,0)]] ConstantBuffer<Frame> frame : register(b0);
[[vk::binding(1,0)]] Texture2D<float4> receivers : register(t0);
[[vk::binding(2,0)]] Texture2D<float4> ambient : register(t1);
[[vk::binding(3,0)]] StructuredBuffer<float> response : register(t1);
[[vk::binding(4,0)]] Texture2D<uint> input_mask : register(t2);
[[vk::binding(5,0)]] [[vk::image_format("r32ui")]] RWTexture2D<uint> output_mask : register(u3);
struct DemandStep {uint4 value;uint4 tile;};
[[vk::binding(6,0)]] ConstantBuffer<DemandStep> step : register(b2);

[shader("compute")][numthreads(8,8,1)]
void build(uint3 id:SV_DispatchThreadID) {
    if(any(id.xy>=step.tile.zw)) return;
    id.xy+=step.tile.xy;
    uint w,h;output_mask.GetDimensions(w,h);
    if(any(id.xy>=uint2(w,h))) return;
    uint occupied=0u;
    if(step.value.z!=0u) {
        uint rw,rh;receivers.GetDimensions(rw,rh);
        let divisor=uint(frame.ao.w);
        let lo=id.xy*divisor;let hi=min(lo+divisor,uint2(rw,rh));
        for(uint y=lo.y;y<hi.y;y++) {for(uint x=lo.x;x<hi.x;x++) {
            let pixel=int3(x,y,0);
            if(receivers.Load(pixel).w>=1.) continue;
            let at=(y*rw+x)*3u;
            occupied |= uint(any(ambient.Load(pixel).rgb!=float3(0.)) ||
                (frame.flags.w>0. && any(float3(response[at],response[at+1u],response[at+2u])!=float3(0.))));
        }}
    } else {
        let p=int2(id.xy);let delta=int2(step.value.xy);
        occupied=input_mask.Load(int3(p,0));
        for(int sign=-1;sign<=1;sign+=2) {
            // Clamp intermediate centres: an out-of-image centre can still
            // have an in-image interval. Discarding it would lose consumers
            // near borders when the next stride exceeds the current radius.
            // The clamped interval stays inside the final clipped support.
            let q=clamp(p+sign*delta,int2(0),int2(w,h)-1);
            occupied |= input_mask.Load(int3(q,0));
        }
    }
    output_mask[id.xy]=occupied;
}
