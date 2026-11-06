#ifdef VORK_SURFACE_MEDIUM
import frame;
import material;
import vork_constants;
#ifdef VORK_SURFACE_MEDIUM_SHARED
import vork_medium_point;
#else
import vork_medium_bounds;
#endif
import vork_medium_types;
import vork_ray_sampling;
import vork_sampling;
#ifdef VORK_SURFACE_MEDIUM_ONLY
import surface_medium_scene;
export Frame medium_frame() {return frame;}
export uint medium_word(uint index) {return material_words[index];}
export Light medium_light(uint index) {return lights[index];}
export float3 medium_environment_radiance(float3 direction) {return reflection_fallback(direction,0.,frame,env_specular,env_sampler);}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint next) {return next;}
export float3 medium_visibility(float3 p,uint index) {return float3(1.);}
#endif
public bool surface_medium_active() {
#ifdef VORK_SURFACE_MEDIUM_SHARED
    return frame.medium.z!=0u && (frame.medium.w&1u)!=0u;
#else
    return true;
#endif
}
public float3 surface_medium_transmittance(float3 origin,float3 direction,float distance) {
#ifdef VORK_SURFACE_MEDIUM_SHARED
    if(!surface_medium_active()) {return float3(1.);}
    let at=frame.medium.z;
    let a=asfloat(uint3(material_words[at],material_words[at+1u],material_words[at+2u]));
    let b=asfloat(uint3(material_words[at+3u],material_words[at+4u],material_words[at+5u]));
    let g=asfloat(uint3(material_words[at+6u],material_words[at+7u],material_words[at+8u]));
    let density=dot(g,origin);let slope=dot(g,direction);let infinite=distance==REG_FP32_MAX;
    let tau=medium_exponential_tau(a,density,slope,distance,infinite)+medium_exponential_tau(b,density,slope,distance,infinite);
    return exp(-min(tau,float3(REG_MEDIUM_EXP_LIMIT)));
#elif VORK_VOLUME_SHADOWS
    let endpoint=distance==REG_FP32_MAX ? medium_infinity() : medium_emitter(distance);
    return medium_transmittance(origin,direction,endpoint);
#else
    return float3(1.);
#endif
}
// One cosine connection per receiver/exposure stratum. A shared surface slot
// must not repeat a full boundary walk for every covered camera sample.
public float3 surface_medium_diffuse(float3 origin,float3 normal,uint sample_index,uint sample_count) {
    let shared=sample_index==0xffffffffu;
    let index=shared ? uint(frame.temporal.x) : sample_index;
    let count=shared ? max(uint(frame.temporal.y),1u) : max(sample_count,1u);
    let pixel=uint2(asuint(origin.x)^asuint(origin.z),asuint(origin.y));
    let disk=ray_concentric_disk(sample_2d(pixel,index,count,REG_SAMPLING_STREAM_MEDIUM_ENVIRONMENT));
    let direction=mul(float3(disk,sqrt(max(1.-dot(disk,disk),0.))),ray_basis(normal));
    let transmission=surface_medium_transmittance(origin,direction,REG_FP32_MAX);
    if(!any(transmission>float3(0.))) {return float3(0.);}
    return transmission*reflection_fallback(direction,0.,frame,env_specular,env_sampler);
}
#endif
