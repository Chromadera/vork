import vork_constants;
import vork_prelude;
[[vk::binding(0, 0)]] Texture2D<float4> receivers : register(t0);
[[vk::binding(1, 0)]] ConstantBuffer<Frame> frame : register(b0);
[[vk::binding(2, 0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> blur_output : register(u1);
[[vk::binding(3, 0)]] Texture2D<float4> raw_ao : register(t2);
[[vk::binding(4, 0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> blur_incident_output : register(u3);
[[vk::binding(5, 0)]] Texture2D<float4> blur_incident_input : register(t4);
// Guide records retain raw visibility variance and the actual beauty count.
[[vk::binding(6,0)]] StructuredBuffer<float4> raw_moments : register(t1);
struct FilterScale {uint4 stride;uint4 tile;};
[[vk::binding(7,0)]] ConstantBuffer<FilterScale> filter_scale : register(b2);
float visibility_variance(int2 pixel,uint width) {
    let guide=raw_moments[(uint(pixel.y)*width+uint(pixel.x))*2u+1u];
    let count=max(guide.z,1.);
    // Finite observations cannot certify zero population variance.
    return max(guide.y,.25/(count*count));
}
uint2 dimensions(RWTexture2D<float4> image) { uint w=0, h=0; image.GetDimensions(w,h); return uint2(w,h); }

// Multiscale incident reconstruction. Averaging variances, rather than squared
// weights, carries a conservative correlation bound into the coarser scales.
void blur_axis(uint2 pixel,int2 axis) {
    let size=dimensions(blur_output);if (any(pixel>=size) ) {return;}
    let p=int2(pixel);let center=raw_ao.Load(int3(p,0));if (center.y<=0. ) {
        blur_output[p]=center;if (frame.flags.w>0. ) {blur_incident_output[p]=float4(0.);}return;
    }
    let normal=decode_octahedron(center.zw);var sum=center.x;var retained=1.;var variance=0.;
    var center_light=float4(0.);if(frame.flags.w>0.) center_light=blur_incident_input.Load(int3(p,0));
    var incident=center_light.rgb;
    let center_visibility_variance=visibility_variance(p,size.x);
    let position=receivers.Load(int3(p,0)).xyz;
    let thickness=max(REG_AO_FILTER_MIN_THICKNESS,center.y*REG_AO_FILTER_DEPTH_SCALE);
    for(var offset=-REG_GI_FILTER_RADIUS;offset<=REG_GI_FILTER_RADIUS;offset++) {
        if(offset==0) continue;
        let q=p+axis*offset*int(filter_scale.stride.x);
        if(any(q<int2(0)) || any(q>=int2(size))) continue;
        let tap=raw_ao.Load(int3(q,0));if(tap.y<=0.) continue;
        let tap_normal=decode_octahedron(tap.zw);
        let delta=receivers.Load(int3(q,0)).xyz-position;
        let tap_thickness=max(REG_AO_FILTER_MIN_THICKNESS,tap.y*REG_AO_FILTER_DEPTH_SCALE);
        let dz=dot(delta,normal)/thickness;
        let reverse_dz=dot(-delta,tap_normal)/tap_thickness;
        var light=float4(0.);if(frame.flags.w>0.) light=blur_incident_input.Load(int3(q,0));
        let difference=light.rgb-center_light.rgb;
        let uncertainty=9.*(max(exp2(center_light.w)-1.,0.)+max(exp2(light.w)-1.,0.))+0.01*(dot(center_light.rgb,center_light.rgb)+dot(light.rgb,light.rgb))/3.+1.e-8;
        let range=exp(-dot(difference,difference)/(3.*uncertainty));
        let visibility_difference=center.x-tap.x;
        let visibility_range=exp(-visibility_difference*visibility_difference/
            max(2.*(center_visibility_variance+visibility_variance(q,size.x)),1.e-8));
        // Symmetric exchange: every pair transfers equal and opposite flux.
        // Missing/blocked neighbours retain their mass at the center. This is
        // doubly stochastic even at image and geometry boundaries.
        let weight=(REG_BINOMIAL5_WEIGHTS[uint(abs(offset))]/16.)*exp(-max(dz*dz,reverse_dz*reverse_dz))*
            pow(max(dot(normal,tap_normal),0.),REG_AO_BLUR_NORMAL_POWER)*range*visibility_range;
        sum+=(tap.x-center.x)*weight;retained-=weight;
        if(frame.flags.w>0.) {incident+=(light.rgb-center_light.rgb)*weight;variance+=max(exp2(light.w)-1.,0.)*weight;}
    }
    blur_output[p]=float4(sum,center.yzw);
    if (frame.flags.w>0. ) {blur_incident_output[p]=float4(incident,log2(1.+variance+retained*max(exp2(center_light.w)-1.,0.)));}
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void horizontal(uint3 gid : SV_DispatchThreadID) {
    if(any(gid.xy>=filter_scale.tile.zw)) return;
    gid.xy+=filter_scale.tile.xy;
    blur_axis(gid.xy,int2(1,0));
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void vertical(uint3 gid : SV_DispatchThreadID) {
    if(any(gid.xy>=filter_scale.tile.zw)) return;
    gid.xy+=filter_scale.tile.xy;
    blur_axis(gid.xy,int2(0,1));
}
