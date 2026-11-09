import vork_visibility;
// One emitter-sampled visibility query, shared by surface and volume lighting.
import vork_constants;
import vork_prelude;
import vork_sampling;
import vork_ray_sampling;
import vork_ray_alpha;
import emitter;
import variant;
#if defined(VORK_FIBRE) && defined(VORK_PROGRAM_COVERAGE)
import vork_fibre_ray;
#endif
internal extern uint shadow_ray_groups(uint index);
internal extern uint shadow_ray_material(uint index);
#ifdef VORK_PROGRAM_COVERAGE
internal extern float3 shadow_ray_fields(RayDesc ray,uint groups,bool all_casters,uint sample_index);
internal extern float3 shadow_ray_contained(RayDesc ray,bool all_casters,uint groups,uint sample_index);
internal extern float3 shadow_ray_candidate(RayIntersection hit,float3x4 world_to_object,RayDesc ray,float distance,uint sample_index,uint sample_count);
#endif
// The view retains separate map-ray and camera offsets in one immutable table.
public float2 ray_emitter_sample(StructuredBuffer<uint> sequence,uint parameters,uint2 pixel,uint light_index,uint sample_index,uint sample_count) {
    if (sample_count<=1u) {return float2(.5);}
    let seed=screen_ray_hash(pixel,0u,REG_SAMPLING_STREAM_SHADOW+light_index*REG_PIXEL_HASH_STREAM_STEP);
    return sample_2d_precomputed(seed,sample_index,sample_count,sequence[parameters+2u+(seed>>16u)%sample_count],sequence[parameters+1u]);
}
import vork_ray_visibility;
public float3 shared_ray_transmission(RaytracingAccelerationStructure ray_scene,StructuredBuffer<uint> ray_alpha,Frame f,LightValues light,uint light_index,uint flags,uint groups,float3 world,float3 normal,uint sample_index,float3 position_error,float2 u,bool fibre=false) {
    if ((flags&REG_LIGHT_RAY_SHADOWED)==0u) {return 1.;}
    let connection=emitter_sample_li<Unspecialized>(light,light_index,world,u,sample_index);
    let receiver=Receiver(world,normal,position_error,0u,sample_index,uint(f.temporal.y)*max(f.sampling.w,1u),1u);
    let policy=ConnectionPolicy(groups,(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,REG_RAY_SHADOW,(flags&REG_LIGHT_RAY_OPAQUE)!=0u);
    return ray_connection_transmittance(ray_scene,ray_alpha,receiver,connection,policy,fibre);
}
// Triangle representation consumes the connection without resampling its source.
public float3 ray_connection_transmittance(RaytracingAccelerationStructure ray_scene,StructuredBuffer<uint> ray_alpha,Receiver receiver,LightConnection connection,ConnectionPolicy policy,bool fibre=false) {
    let world=receiver.world;let normal=receiver.normal;let position_error=receiver.position_error;
    let direction=connection.direction;let distance=connection.distance;let sample_index=receiver.sample_index;
    let origin=ray_origin(world,normal,direction,position_error);
    let all_casters=policy.all_casters;
    // Visit every eligible crossing, including null sheets in otherwise opaque
    // geometry. Only zero RGB throughput ends the connection early.
    let ray_flags=RAY_FLAG_FORCE_NON_OPAQUE;
    RayDesc ray;ray.Origin=origin;ray.TMin=0.;ray.Direction=direction;ray.TMax=max(distance-length(origin-world),0.);
    // Candidate coverage decisions index the exposure's samples: its scene-time
    // strata times the visibility samples each integrates in decoupled colour
    // (sampling.w, 0 elsewhere), as sample_index does.
    let budget=receiver.sample_count;
#if defined(VORK_FIBRE) && defined(VORK_PROGRAM_COVERAGE)
    if (fibre) {return fibre_ray_visibility(ray_scene,ray,all_casters,policy.groups,sample_index,budget)*shadow_ray_contained(ray,all_casters,policy.groups,sample_index);}
#endif
    var fields=float3(1.);
#ifdef VORK_PROGRAM_COVERAGE
    fields=shadow_ray_fields(ray,policy.groups,all_casters,sample_index);
#endif
    if(!any(fields>float3(0.))) return float3(0.);
    if (policy.proven_opaque) {return fields*select(opaque_shadow_hit(ray_scene,origin,direction,ray.TMax,REG_RAY_SHADOW),0.,1.);}
    var result=ConnectionVisibility(fields,0u);
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(ray_scene,ray_flags,policy.ray_mask,ray);
    while (query.Proceed()) {
        let hit=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        if (!all_casters && !links_intersect(shadow_ray_groups(hit.instance_custom_data),policy.groups)) {continue;}
#ifdef VORK_PROGRAM_COVERAGE
        let transmission=shadow_ray_candidate(hit,query.CandidateWorldToObject3x4(),ray,query.CandidateTriangleRayT(),sample_index,budget);
#else
        let transmission=float3(1.-ray_candidate_opacity(ray_alpha,hit,shadow_ray_material(hit.instance_custom_data)));
#endif
        result.transmittance*=transmission;
        if (!any(result.transmittance>float3(0.))) {return float3(0.);}
    }
#ifdef VORK_PROGRAM_COVERAGE
    result.transmittance*=shadow_ray_contained(ray,all_casters,policy.groups,sample_index);
#endif
    return result.transmittance;
}
