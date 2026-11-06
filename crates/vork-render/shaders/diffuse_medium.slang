#ifdef VORK_DIFFUSE_MEDIUM
import vork_medium_types;
import vork_constants;
// Source-only medium transport for diffuse connections. Emission is integrated
// once, at the source generation; later probe generations only attenuate their
// reflected radiance. Domain traversal and coefficient integration are shared
// with camera media, including overlapping and heterogeneous finite domains.
import frame;
import material;
import vork_medium_bounds;
import diffuse_medium_scene;
__exported import medium_source_proposal;
internal extern bool volume_emitting(uint input);
export Frame medium_frame() {return frame;}
export uint medium_word(uint index) {return material_words[index];}
// Source generation includes authored emission and local single scattering.
// Later probe generations still request attenuation alone.
// Select one analytic source per scattering event. The transport service applies
// its inverse discrete PDF after clamping to preserve the local-light integral without
// a per-light visibility loop inside each secondary lighting cell.
import vork_sampling;
export Light medium_light(uint index) {return lights[index];}
export float3 medium_environment_radiance(float3 direction) {return reflection_fallback(direction,0.,frame,env_specular,env_sampler);}
export uint medium_light_list(float3 position,out uint base) {
    let count=uint(frame_light_count(frame));base=0u;if(count==0u) {return 0u;}
    let pixel=uint2(asuint(position.x)^asuint(position.z),asuint(position.y));
    let u=sample_2d(pixel,uint(frame.temporal.x),max(uint(frame.temporal.y),1u),REG_SAMPLING_STREAM_MEDIUM_LIGHT).x;
    base=min(uint(u*float(count)),count-1u);return 1u;
}
export uint medium_light_index(uint base,uint next) {return base;}
// The consumer supplies its existing ray table and material dispatcher. These
// connections evaluate visibility and source emission only, never diffuse
// lighting, so they cannot recursively launch another medium source integral.
import surface;
import hit_geometry;
import hit_lighting;
public float3 diffuse_source_visibility<P:IProgram,G:IHitGeometry>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,float3 position,uint index) {
    return hit_shadow<P,G>(scene,words,index,position,float3(0.),float3(0.),uint(frame.temporal.x),uint(frame.temporal.y));
}
#ifdef VORK_MEDIUM_MESH_SOURCES
import vork_medium_mesh;
import vork_medium_shadow;
public float3 diffuse_mesh_illumination<P:IProgram,G:IHitGeometry>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,StructuredBuffer<float> weights,float3 position,float3 ray,float g) {
    return medium_mesh_incident<P,G>(scene,words,weights,frame.medium.y,position,ray,g,medium_shadow_pixel(position),uint(frame.temporal.x),uint(frame.temporal.y));
}
#endif
export uint medium_source_domains() {return volume_domain_count();}
export bool medium_source_domain(uint index) {
    let at=medium_frame().medium.x+medium_word(medium_frame().medium.x)+index*REG_MEDIUM_BOUND_WORDS;
    return (objects[medium_word(at)].metadata.y&REG_RAY_VOLUME)!=0u && volume_emitting(medium_frame().medium.x+medium_word(at+4u));
}
export float medium_source_cone(uint index,float3 from,out float3 axis) {
    let at=medium_frame().medium.x+medium_word(medium_frame().medium.x)+index*REG_MEDIUM_BOUND_WORDS;
    let model=object_model(medium_word(at));let root=medium_frame().medium.x+medium_word(at+1u);
    let lo=volume_vec3(root);let hi=volume_vec3(root+4u);let local_center=(lo+hi)*.5;
    let center=mul(float4(local_center,1.),model).xyz;var squared_radius=0.;
    for(uint corner=0u;corner<8u;corner++) {
        let local=lerp(lo,hi,float3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u))-local_center;
        let delta=mul(float4(local,0.),model).xyz;squared_radius=max(squared_radius,dot(delta,delta));
    }
    let delta=center-from;let squared_distance=dot(delta,delta);
    axis=squared_distance>0. ? delta/sqrt(squared_distance) : float3(0.,0.,1.);
    if(squared_distance<=squared_radius) {return 2.;}
    let sine_squared=min(squared_radius/squared_distance,1.);
    return sine_squared/(1.+sqrt(1.-sine_squared));
}

public float3 diffuse_medium_radiance(float3 origin,float3 direction,float distance,float3 radiance,float fraction,bool source=true) {
    let endpoint=distance==REG_FP32_MAX ? medium_infinity() : medium_receiver(distance);
    if(!source) {return volume_finite<VORK_MEDIUM_STEPS,false>(origin,direction,endpoint,fraction,0xffffffffu,false).transmission*radiance;}
    let transport=volume_finite<VORK_MEDIUM_STEPS,true>(origin,direction,endpoint,fraction,0xffffffffu,false);
    return transport.transmission*radiance+(source ? transport.source : float3(0.));
}

#endif
