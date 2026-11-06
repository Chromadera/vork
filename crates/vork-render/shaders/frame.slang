import vork_constants;
__exported import vork_prelude;
__exported import vork_visibility;
__exported import vork_sampling;
__exported import vork_light_falloff;
#ifdef VORK_VOLUME_BINDINGS
[[vk::binding(5,0)]] ConstantBuffer<Frame> frame;
[[vk::binding(46,0)]] StructuredBuffer<Object> objects;
#else
[[vk::binding(0,0)]] ConstantBuffer<Frame> frame : register(b0);
[[vk::binding(1,0)]] StructuredBuffer<Object> objects : register(t1);
#endif
// Matrices in the WGSL convention (Slang row i = WGSL column i), so mul(v,m) = M*v.
float4x4 affine_matrix(float4 rows[3]) {return transpose(float4x4(rows[0],rows[1],rows[2],float4(0.,0.,0.,1.)));}
float4x4 object_model(uint object) {return affine_matrix(objects[object].model);}
bool collapsed_endpoint(float4 rows[3]) {
    let m=affine_matrix(rows);
    return dot(m[0].xyz,cross(m[1].xyz,m[2].xyz))==0.;
}
float4x4 visible_endpoint(uint object,float4 rows[3]) {
    let m=affine_matrix(rows);
    if (collapsed_endpoint(rows)) {return object_model(object);}
    return m;
}
float4x4 object_previous_model(uint object) {return visible_endpoint(object,objects[object].previous_model);}
float4x4 object_next_model(uint object) {return visible_endpoint(object,objects[object].next_model);}
float4x4 object_normal(uint object) {return object_normal_matrix(objects[object]);}
// Volume shadow closures share the surface BRDF/layer lookups. Other volume
// entries need only frame/object context.
#ifndef VORK_VOLUME_BINDINGS
#ifndef VORK_FRAME_LIGHTS_REGISTER
#define VORK_FRAME_LIGHTS_REGISTER t2
#endif
[[vk::binding(2,0)]] StructuredBuffer<Light> lights : register(VORK_FRAME_LIGHTS_REGISTER);
[[vk::binding(3,0)]] Texture2D env_specular : register(t0);
[[vk::binding(4,0)]] SamplerState env_sampler : register(s0);
[[vk::binding(5,0)]] Texture2D env_diffuse : register(t1);
[[vk::binding(6,0)]] Texture2D env_brdf : register(t2);
[[vk::binding(7,0)]] ConstantBuffer<ClusterConfig> clusters : register(b3);
[[vk::binding(8,0)]] StructuredBuffer<uint> cluster_lights : register(t4);
[[vk::binding(9,0)]] Texture2D ltc_amplitude : register(t3);

#endif

#if !defined(VORK_VOLUME_BINDINGS) || defined(VORK_PROGRAM_COVERAGE)
#ifdef VORK_VOLUME_BINDINGS
[[vk::binding(12,0)]] SamplerState env_sampler;
[[vk::binding(13,0)]] Texture2D env_brdf;
[[vk::binding(18,0)]] Texture2D ltc_amplitude;
#endif
float4 brdf_texel(float2 uv) {
    let size=float(REG_BRDF_SIZE);
    // Inverse of the bake's fourth-power cosine / logarithmic physical-alpha grid.
    uv.x=sqrt(sqrt(clamp(uv.x,0.,1.)));
    uv.y=log2(clamp(uv.y*uv.y,REG_GGX_ALPHA_MIN,1.)/REG_GGX_ALPHA_MIN)/log2(1./REG_GGX_ALPHA_MIN);
    uv=(uv*(size-1.)+.5)/size;
    uv.y*=size/float(REG_BRDF_HEIGHT);
    return env_brdf.SampleLevel(env_sampler,uv,0.);
}
float3 brdf_moments(float2 uv) {return brdf_texel(uv).rgb;}
float brdf_average_albedo(float roughness) {return brdf_texel(float2(0.,roughness)).a;}
float dielectric_albedo(float nv, float roughness, float eta) {
    if (eta==1.) {return 0.;}
    let at=eta/(1.+eta)*float(REG_BRDF_IOR_STEPS);
    let first=min(uint(at),REG_BRDF_IOR_STEPS-1u);
    let n=REG_BRDF_ALBEDO_SIZE;
    let local=.5+clamp(float2(nv,roughness),float2(0.),float2(1.))*float(n-1u);
    let extent=float2(REG_BRDF_SIZE,REG_BRDF_HEIGHT);
    let a=float2((first%2u)*n,REG_BRDF_SIZE+(first/2u)*n);
    let second=first+1u;
    let b=float2((second%2u)*n,REG_BRDF_SIZE+(second/2u)*n);
    return lerp(env_brdf.SampleLevel(env_sampler,(a+local)/extent,0.).x,
        env_brdf.SampleLevel(env_sampler,(b+local)/extent,0.).x,at-float(first));
}
#endif
