import vork_constants;
// Opaque topology needs no candidate callbacks. Metal's intersector lets the
// hardware own traversal instead of suspending it in an inline query.
public bool opaque_shadow_hit(RaytracingAccelerationStructure scene,float3 origin,float3 direction,float distance,uint mask) {
#ifdef VORK_METAL
    __intrinsic_asm "([](raytracing::instance_acceleration_structure s,float3 o,float3 d,float t,uint m) { raytracing::intersector<raytracing::instancing> i; i.accept_any_intersection(true); i.force_opacity(raytracing::forced_opacity::opaque); return i.intersect(raytracing::ray(o,d,0.0f,t),s,m).type != raytracing::intersection_type::none; }($0,$1,$2,$3,$4))";
#else
    RayDesc ray;ray.Origin=origin;ray.TMin=0.;ray.Direction=direction;ray.TMax=distance;
    RayQuery<RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;
    query.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);
    while (query.Proceed()) {}
    return query.CommittedStatus()!=COMMITTED_NOTHING;
#endif
}
