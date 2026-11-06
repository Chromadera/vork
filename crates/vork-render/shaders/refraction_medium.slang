#ifdef VORK_BOUNDED_MEDIUM
// Bind the existing medium segment service to the camera's shared frame, words,
// light records and retained visibility field. Coefficient transport stays shared.
import frame;
import variant;
import vork_constants;
import vork_material_table;
import vork_medium;
import vork_medium_visibility_direct;
import vork_medium_shadow;
import vork_shadow_visibility;
import rays;
import vork_sampling;

import refraction_medium_scene;
static uint medium_sample_index;
static uint medium_sample_count=1u;
public void refraction_medium_sample(uint index,uint count) {medium_sample_index=index;medium_sample_count=count;}
export float3 medium_visibility(float3 position,uint index) {
#ifdef VORK_MEDIUM_DIRECT_RAY
    let flags=lights[index].shadow_offsets[REG_LIGHT_RAY_WORD];
    if((flags&REG_LIGHT_RAY_SHADOWED)!=0u) {
        return ray_visibility(position,float3(0.),light_values<Unspecialized>(index),index,medium_shadow_pixel(position),float3(0.),medium_sample_index);
    }
#endif
    return float3(1.);
}
#ifdef VORK_MEDIUM_MESH_SOURCES
import vork_medium_mesh;
import vork_medium_shadow;
import rays;
import scene_programs;
import ray_coverage;
export float3 medium_mesh_illumination(float3 position,float3 ray,float g) {
    let f=frame;
    return medium_mesh_incident<ScenePrograms,ReceiverGeometry>(ray_scene,ray_alpha,receiver_emitter_weights,f.medium.y,position,ray,g,medium_shadow_pixel(position),medium_sample_index,medium_sample_count);
}
#endif
export uint medium_word(uint index) {return material_words[index];}
export Light medium_light(uint index) {return lights[index];}
export float3 medium_environment_radiance(float3 direction) {return reflection_fallback(direction,0.,frame,env_specular,env_sampler);}
// A secondary interval selects one unculled analytic source. The transport
// service applies its inverse PDF after the authored per-source light clamp,
// so neither light count nor a boundary continuation multiplies fog walks.
export uint medium_light_list(float3 position,out uint base) {
    let count=uint(frame_light_count(frame));base=0u;if(count==0u) {return 0u;}
    let pixel=uint2(asuint(position.x)^asuint(position.z),asuint(position.y));
    let u=sample_2d(pixel,medium_sample_index,max(medium_sample_count,1u),REG_SAMPLING_STREAM_MEDIUM_LIGHT).x;
    base=min(uint(u*float(count)),count-1u);return 1u;
}
export uint medium_light_index(uint base,uint next) {return base;}

#endif
