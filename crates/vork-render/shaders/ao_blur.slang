import vork_constants;
import vork_prelude;
[[vk::binding(0, 0)]] Texture2D<float4> receivers : register(t0);
[[vk::binding(1, 0)]] ConstantBuffer<Frame> frame : register(b0);
[[vk::binding(2, 0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> blur_output : register(u1);
[[vk::binding(3, 0)]] Texture2D<float4> raw_ao : register(t2);
[[vk::binding(4, 0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> blur_incident_output : register(u3);
[[vk::binding(5, 0)]] Texture2D<float4> blur_incident_input : register(t4);
uint2 dimensions(RWTexture2D<float4> image) { uint w=0, h=0; image.GetDimensions(w,h); return uint2(w,h); }

// Separable per-camera-sample bilateral reconstruction.
void blur_axis(uint2 pixel,int2 axis) {
    let size=dimensions(blur_output);if (any(pixel>=size) ) {return;}
    let p=int2(pixel);let center=raw_ao.Load(int3(p,0));if (center.y<=0. ) {
        blur_output[p]=center;if (frame.flags.w>0. ) {blur_incident_output[p]=float4(0.);}return;
    }
    let normal=decode_octahedron(center.zw);var sum=0.;var weights=0.;var incident=float3(0.);
    let position=receivers.Load(int3(p,0)).xyz;
    let thickness=max(REG_AO_FILTER_MIN_THICKNESS,center.y*REG_AO_FILTER_DEPTH_SCALE);
    for(var offset=-REG_AO_BLUR_RADIUS;offset<=REG_AO_BLUR_RADIUS;offset++) {
        let q=clamp(p+axis*offset,int2(0),int2(size)-1);let tap=raw_ao.Load(int3(q,0));if (tap.y<=0. ) {continue;}
        var dz=0.;
        if (offset!=0) {dz=dot(receivers.Load(int3(q,0)).xyz-position,normal)/thickness;}
        let weight=REG_BINOMIAL5_WEIGHTS[uint(abs(offset))]*exp(-dz*dz)*pow(max(dot(normal,decode_octahedron(tap.zw)),0.),REG_AO_BLUR_NORMAL_POWER);
        sum+=tap.x*weight;weights+=weight;
        if (frame.flags.w>0. ) {incident+=blur_incident_input.Load(int3(q,0)).rgb*weight;}
    }
    blur_output[p]=float4(sum/max(weights,REG_POSITIVE_EPSILON),center.yzw);
    if (frame.flags.w>0. ) {blur_incident_output[p]=float4(incident/max(weights,REG_POSITIVE_EPSILON),0.);}
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void horizontal(uint3 gid : SV_DispatchThreadID) {
    blur_axis(gid.xy,int2(1,0));
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void vertical(uint3 gid : SV_DispatchThreadID) {
    blur_axis(gid.xy,int2(0,1));
}
