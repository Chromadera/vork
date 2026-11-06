import vork_visibility;
// Physical-fibre receiver visibility. Resolve the closest linked boundary before
// evaluating its closure, then resume after a transmitting boundary. Material
// evaluation does not run inside a live candidate traversal. No crossing budget.
import vork_constants;
import vork_prelude;
import vork_ray_alpha;
internal extern uint shadow_ray_groups(uint index);
internal extern float3 shadow_ray_fibre_candidate(RayIntersection hit,float3x4 inverse,RayDesc ray,float distance,uint sample_index,uint sample_count);
public float3 fibre_ray_visibility(RaytracingAccelerationStructure scene,RayDesc ray,bool all_casters,uint groups,uint sample_index,uint sample_count) {
    var transmission=float3(1.);
    for (;;) {
        RayIntersection hit;
        float3x4 inverse;
        float distance;
        {
            RayQuery<RAY_FLAG_NONE> query;
            // An all-caster query can find its closest boundary directly. Linked
            // queries commit only eligible candidates before resolving closest.
            let flags=select(all_casters,RAY_FLAG_FORCE_OPAQUE,RAY_FLAG_FORCE_NON_OPAQUE);
            query.TraceRayInline(scene,flags,REG_RAY_SHADOW,ray);
            while (query.Proceed()) {
                if (links_intersect(shadow_ray_groups(query.CandidateInstanceID()),groups)) {query.CommitNonOpaqueTriangleHit();}
            }
            if (query.CommittedStatus()==COMMITTED_NOTHING) {return transmission;}
            hit=RayIntersection(query.CommittedInstanceIndex(),query.CommittedInstanceID(),query.CommittedPrimitiveIndex(),query.CommittedTriangleBarycentrics());
            inverse=query.CommittedWorldToObject3x4();
            distance=query.CommittedRayT();
        }
        transmission*=shadow_ray_fibre_candidate(hit,inverse,ray,distance,sample_index,sample_count);
        if (all(transmission==float3(0.))) {return transmission;}
        // Ray t is nonnegative. The next representable value makes strict
        // progress without a world-space epsilon or skipping another boundary.
        ray.TMin=asfloat(asuint(distance)+1u);
        if (ray.TMin>=ray.TMax) {return transmission;}
    }
}
