// Shared OIDN workspace: three float4 planes (RGB image stride 16).
import vork_constants;
struct Params { uint count; uint samples; uint guide_alpha; uint width; uint spectral; uint channel; float input_scale; };
[[vk::binding(0,0)]] ConstantBuffer<Params> params : register(b0);
[[vk::binding(1,0)]] RWStructuredBuffer<float4> accumulation : register(u1);
[[vk::binding(2,0)]] StructuredBuffer<float4> guides : register(t2);
[[vk::binding(3,0)]] RWStructuredBuffer<float4> shared : register(u3);
[[vk::binding(4,0)]] RWStructuredBuffer<float4> output : register(u4);

float3 spectral_color(float3 rgb,bool inverse) {
    if(inverse) return float3(
        dot(rgb,float3(DENOISE_FROM_XYZ_00,DENOISE_FROM_XYZ_01,DENOISE_FROM_XYZ_02)),
        dot(rgb,float3(DENOISE_FROM_XYZ_10,DENOISE_FROM_XYZ_11,DENOISE_FROM_XYZ_12)),
        dot(rgb,float3(DENOISE_FROM_XYZ_20,DENOISE_FROM_XYZ_21,DENOISE_FROM_XYZ_22)));
    return float3(
        dot(rgb,float3(DENOISE_TO_XYZ_00,DENOISE_TO_XYZ_01,DENOISE_TO_XYZ_02)),
        dot(rgb,float3(DENOISE_TO_XYZ_10,DENOISE_TO_XYZ_11,DENOISE_TO_XYZ_12)),
        dot(rgb,float3(DENOISE_TO_XYZ_20,DENOISE_TO_XYZ_21,DENOISE_TO_XYZ_22)));
}

[shader("compute")]
[numthreads(DENOISE_GROUP,1,1)]
void pack(uint3 id : SV_DispatchThreadID) {
    if(id.x>=params.width) return;
    uint i=id.y*params.width+id.x; if(i>=params.count) return;
    float4 p=accumulation[i]/float(params.samples);
    shared[i]=float4(p.a>0. ? p.rgb/p.a : float3(0.),0.);
    float coverage=params.guide_alpha!=0 ? guides[2*i].a : float(params.samples);
    shared[params.count+i]=float4(coverage>0. ? guides[2*i].rgb/coverage : float3(0.),0.);
    if(params.spectral!=0u) {
        shared[i].rgb=spectral_color(shared[i].rgb,false);
        shared[params.count+i].rgb=spectral_color(shared[params.count+i].rgb,false);
        shared[i].rgb=float3(shared[i][params.channel]);
        shared[params.count+i].rgb=float3(shared[params.count+i][params.channel]);
    }
    shared[i].rgb*=params.input_scale;
    float3 n=coverage>0. ? guides[2*i+1].rgb/coverage : float3(0.);
    // Accumulate unit normals, then retain the anti-aliased average length.
    if(params.channel==0u) shared[2*params.count+i]=float4(n,0.);
}

[shader("compute")]
[numthreads(DENOISE_GROUP,1,1)]
void unpack(uint3 id : SV_DispatchThreadID) {
    if(id.x>=params.width) return;
    uint i=id.y*params.width+id.x; if(i>=params.count) return;
    float4 p=accumulation[i]/float(params.samples);
    if(params.spectral!=0u) {
        output[i][params.channel]=(shared[i].r+shared[i].g+shared[i].b)/(3.*params.input_scale);
        if(params.channel<2u) return;
        if(guides[2*i].a>0.) p.rgb=spectral_color(output[i].rgb,true)*p.a;
    } else if(guides[2*i].a>0.) p.rgb=(shared[i].rgb/params.input_scale)*p.a;
    output[i]=p;
    accumulation[i]=p*float(params.samples);
}
