// Closest geometric triangle boundary; no material or coverage callbacks.
__exported import program_trace;
import hit;
import material;
import frame;
import vork_constants;
void trace_failure(uint reason) {InterlockedMax(shader_errors[0],REG_TRACE_EXHAUSTED);InterlockedMin(shader_errors[1],reason);}
TraceHit geometric_trace<G:IHitGeometry>(RaytracingAccelerationStructure scene,float3 origin,float3 direction,float minimum,float maximum,uint mask,uint object) {
    TraceHit h={};h.distance=-1.;h.object_id=0xffffffffu;h.instance_index=0xffffffffu;h.primitive_id=0xffffffffu;
    let scale=max(abs(direction.x),max(abs(direction.y),abs(direction.z)));
    let magnitude=length(direction/scale);
    if((mask&255u)==0u || !all(isfinite(origin)) || !all(isfinite(direction)) || !(scale>0.) || !isfinite(magnitude) || !isfinite(minimum) || !isfinite(maximum) || minimum<0. || maximum<minimum) {return h;}
    let axis=(direction/scale)/magnitude;
    uint ray_index;InterlockedAdd(shader_errors[REG_TRACE_RAYS_WORD],1u,ray_index);
    if(ray_index>=REG_TRACE_RAY_LIMIT) {trace_failure(1u);return h;}
    RayDesc ray;ray.Origin=origin;ray.Direction=axis;ray.TMin=minimum;ray.TMax=maximum;
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(scene,object==0xffffffffu ? RAY_FLAG_FORCE_OPAQUE : RAY_FLAG_FORCE_NON_OPAQUE,mask&255u,ray);
    uint work=0u;
    while(query.Proceed()) {
        if(work++>=REG_TRACE_TRAVERSAL_LIMIT) {query.Abort();trace_failure(2u);return h;}
        if(query.CandidateType()==CANDIDATE_NON_OPAQUE_TRIANGLE && (object==0xffffffffu || uint(objects[query.CandidateInstanceID()].opacity.y)==object)) {query.CommitNonOpaqueTriangleHit();}
    }
    if(query.CommittedStatus()!=COMMITTED_TRIANGLE_HIT) {return h;}
    h.hit=true;h.distance=query.CommittedRayT();h.position=origin+axis*h.distance;
    h.object_id=query.CommittedInstanceID();h.instance_index=query.CommittedInstanceIndex();h.primitive_id=query.CommittedPrimitiveIndex();
    let uv=query.CommittedTriangleBarycentrics();h.barycentrics=float3(1.-uv.x-uv.y,uv);
    let triangle=hit_triangle<G>(h.instance_index,h.primitive_id,h.object_id,query.CommittedWorldToObject3x4());
    h.geometric_normal=hit_face(triangle);
    let varying=hit_varying(triangle,h.barycentrics);
    h.shading_normal=dot(varying.NORMAL,varying.NORMAL)>0. ? normalize(varying.NORMAL) : h.geometric_normal;h.uv=varying.TEXCOORD_0;
    h.front_facing=dot(axis,h.geometric_normal)<0.;
    let record=objects[h.object_id];h.object_id=uint(record.opacity.y);
    h.instance_index=record.padding.y;h.primitive_id=material_words[record.reserved+14u+h.primitive_id];return h;
}
