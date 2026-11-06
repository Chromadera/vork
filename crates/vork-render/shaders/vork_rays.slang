// Hit kernels own their query globals; imported raster helpers are eliminated.
#ifdef VORK_HIT_KERNEL
#define RASTER_REGISTER(slot)
#else
#define RASTER_REGISTER(slot) : register(slot)
#endif
import vork_constants;
import light_connection;
#ifdef VORK_CAMERA_SPLIT
[[vk::binding(14,4)]] public StructuredBuffer<float> receiver_emitter_weights : register(t29);
#endif
import vork_sampling;
[[vk::binding(0,2)]] public StructuredBuffer<uint> shadow_sequence RASTER_REGISTER(t10);
import frame;
import vork_ray_sampling;
import vork_ray_alpha;
#ifdef VORK_RAY_BINDINGS
[[vk::binding(1,2)]] public RaytracingAccelerationStructure ray_scene RASTER_REGISTER(t11);
[[vk::binding(2,2)]] public StructuredBuffer<uint> ray_alpha RASTER_REGISTER(t12);
#else
[[vk::binding(1,2)]] public RaytracingAccelerationStructure ray_scene;
[[vk::binding(2,2)]] public StructuredBuffer<uint> ray_alpha;
#endif
export float3 ray_eye() {return frame.eye.xyz;}
import vork_shadow_ray;
export uint shadow_ray_groups(uint index) {return objects[index].metadata.x;}
export uint shadow_ray_material(uint index) {return objects[index].material_index;}
// One emitter ray for sample `sample_index` (the exposure sample unless given) of
// the frame's ray sequence, whose header holds its length.
public float3 ray_transmission(float3 world,float3 normal,LightValues light,uint light_index,uint2 pixel,float3 position_error,bool fibre=false,uint sample_override=0xffffffffu) {
    if ((lights[light_index].shadow_offsets[REG_LIGHT_RAY_WORD]&REG_LIGHT_RAY_SHADOWED)==0u) {return float3(1.);}
    let sample_index=select(sample_override==0xffffffffu,uint(frame.temporal.x),sample_override);
    let parameters=frame.integrator.w>>REG_INTEGRATOR_RAY_SEQUENCE_SHIFT;
    let u=ray_emitter_sample(shadow_sequence,parameters,pixel,light_index,sample_index,shadow_sequence[parameters]);
    return shared_ray_transmission(ray_scene,ray_alpha,frame,light,light_index,lights[light_index].shadow_offsets[REG_LIGHT_RAY_WORD],lights[light_index].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],world,normal,sample_index,position_error,u,fibre);
}
// Use the exact connection selected by the BSDF/source estimator.
public float3 ray_visibility_direction(float3 world,float3 normal,uint light,float3 direction,float distance,float3 error,uint index,uint count,bool fibre=false) {
    let flags=lights[light].shadow_offsets[REG_LIGHT_RAY_WORD];
    if((flags&REG_LIGHT_RAY_SHADOWED)==0u) {return float3(1.);}
    LightConnection connection={};connection.direction=direction;connection.distance=distance;
    let receiver=Receiver(world,normal,error,0u,index,count,1u);
    let policy=ConnectionPolicy(lights[light].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,REG_RAY_SHADOW,(flags&REG_LIGHT_RAY_OPAQUE)!=0u);
    return ray_connection_transmittance(ray_scene,ray_alpha,receiver,connection,policy,fibre);
}
// Ordinary receivers consume the same RGB straight-segment transmittance.
public float3 ray_visibility(float3 world,float3 normal,LightValues light,uint light_index,uint2 pixel,float3 position_error=float3(0.),uint sample_override=0xffffffffu) {
    return ray_transmission(world,normal,light,light_index,pixel,position_error,false,sample_override);
}

#if defined(VORK_PROGRAM_COVERAGE) || defined(VORK_PROBES) || defined(VORK_RAY_REFRACTION)
import hit;
// Like the TLAS: a fixed raster register only where the shadow group binds it
// (ray shadows), so a kernel without it numbers its own resources unshifted.
#if defined(VORK_METAL) && defined(VORK_RAY_BINDINGS)
[[vk::binding(3,2)]] ParameterBlock<RayHitBuffersTable> ray_hit_buffers RASTER_REGISTER(b13);
#elif defined(VORK_METAL)
[[vk::binding(3,2)]] ParameterBlock<RayHitBuffersTable> ray_hit_buffers;
#elif defined(VORK_RAY_BINDINGS)
[[vk::binding(3,2)]] StructuredBuffer<uint> ray_hit_buffers[REG_RAY_HIT_BUFFERS] RASTER_REGISTER(t13);
#else
[[vk::binding(3,2)]] StructuredBuffer<uint> ray_hit_buffers[REG_RAY_HIT_BUFFERS];
#endif
uint ray_hit_buffers_load(uint buffer,uint index) {
#ifdef VORK_METAL
    return ray_hit_buffers.buffers[buffer][index];
#else
    return ray_hit_buffers[NonUniformResourceIndex(buffer)][index];
#endif
}
public struct ReceiverGeometry : IHitGeometry {
    static uint word(uint index) {return ray_alpha[index];}
    static uint load(uint buffer,uint index) {return ray_hit_buffers_load(buffer,index);}
};
#endif
#ifdef VORK_PROGRAM_COVERAGE
import variant;
import scene_programs;
import ray_coverage;
import shadow_coverage;
import light_connection;
export float3 shadow_ray_fields(RayDesc ray,uint groups,bool all_casters,uint sample_index) {
    return field_shadow_transmittance<ScenePrograms,ReceiverGeometry>(ray_alpha,ray,groups,all_casters,sample_index);
}
export float3 shadow_ray_contained(RayDesc ray,bool all_casters,uint groups,uint sample_index) {
    return shadow_contained_transmittance<ScenePrograms,ReceiverGeometry>(ray_scene,ray_alpha,ray,all_casters,groups,frame.lens.zw,sample_index);
}
export float3 shadow_ray_candidate(RayIntersection hit,float3x4 world_to_object,RayDesc ray,float distance,uint sample_index,uint sample_count) {
    return shadow_candidate_transmittance<ScenePrograms,ReceiverGeometry>(ray_scene,ray_alpha,hit,world_to_object,ray,distance,frame.lens.zw,sample_index);
}
#ifdef VORK_FIBRE
import hair_lighting;
export float3 shadow_ray_fibre_candidate(RayIntersection hit,float3x4 inverse,RayDesc ray,float distance,uint sample_index,uint sample_count) {
    let direction=ray.Direction;
    let surface=hit_surface<Unspecialized,ScenePrograms,ReceiverGeometry>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,inverse,hit.barycentrics,direction,frame.lens.zw);
    if (surface.s.is_hair) {
        if (distance>ray.TMax) {return float3(1.);}
        return surface.entering ? strand_forward_at(surface.s.hair0,surface.s.hair1,surface.s.strand,surface.s.geometric,-direction) : float3(1.);
    }
    return shadow_candidate_transmittance<ScenePrograms,ReceiverGeometry,false,false>(ray_scene,ray_alpha,hit,inverse,ray,distance,frame.lens.zw,sample_index);
}
#endif
#endif

import subsurface_transport;
public float ray_subsurface_exit(uint object,float3 world,float3 direction) {return subsurface_exit(ray_scene,object,world,direction);}

#ifdef VORK_PROBES
import surface;
import ray_coverage;
public bool ray_probe_visible(float3 world,float3 normal,float3 position_error,float3 probe) {
#ifdef VORK_PROGRAM_COVERAGE
    return probe_connection_visible<ScenePrograms,ReceiverGeometry>(ray_scene,ray_alpha,world,normal,position_error,probe);
#else
    return probe_connection_visible<NoPrograms,ReceiverGeometry>(ray_scene,ray_alpha,world,normal,position_error,probe);
#endif
}
#endif
