import catcher;
// Scene boundary services; callers supply the shared ray table and hit geometry.
import vork_constants;
import frame;
import material;
import hit;
import implicit_scene;
import hit_geometry;
import variant;
import surface;
import stream;
import closure;
import path_boundary;
#include "path_work_arguments.slang"
struct DielectricHit { float t; uint instance; uint primitive; uint object; float2 barycentrics; float3x4 world_to_object; bool field; FieldHit field_hit; };
// Merge field zero sets with an existing hardware triangle hit. Both camera
// path transport and raster dielectric continuation use the same acceptance,
// visibility masks and contact tie rule. Domain-box triangles never shade.
#ifdef VORK_FIELD_OUTLINE
[noinline]
#endif
DielectricHit merge_field_hits<P:IProgram,G:IHitGeometry>(DielectricHit h,uint2 fields,float3 origin,float3 direction,float distance,uint mask,uint only_object=0xffffffffu) {
    for(uint i=0u;i<fields.y;i++) {
        let at=fields.x+i*3u;let instance=G.word(at);let object=G.word(at+1u);
        if((only_object!=0xffffffffu && object!=only_object) || !catcher_keeps(object) || (objects[object].metadata.y&mask)==0u || collapsed_endpoint(objects[object].model)) continue;
        var minimum=0.;let maximum=h.t>=0. ? h.t : distance;
        for(;;) {
            let root=field_intersection<G>(G.word(at+2u),object,origin,direction,minimum,maximum);
            if(!root.found) break;
            if(h.t>=0. && !h.field) {
                // Resolve a numerically coincident contact in favour of the
                // triangle. Accepting the field exit instead can offset the
                // continuation through an opaque receiver touching the liquid.
                // Use the existing world-position bounds, not a scene epsilon.
                let field_error=surface_position_error(object,origin+direction*root.t);
                let triangle_error=surface_position_error(h.object,origin+direction*h.t);
                if(h.t-root.t<=length(field_error)+length(triangle_error)) break;
            }
            let surface=field_surface<Unspecialized,P>(object,root,direction,float2(0.),Lobe(0u,0.));
            if(!surface.s.discarded && surface.s.closure.geometry_opacity>0.) {
                h.field=true;h.field_hit=root;h.t=root.t;h.instance=instance;h.object=object;
                h.world_to_object=(float3x4)object_normal(object);break;
            }
            minimum=root.t+REG_FP32_GAMMA7*max(1.,abs(root.t));
            if(minimum>=maximum) break;
        }
    }
    return h;
}
HitSurface dielectric_surface<V:IVariant,P:IProgram,G:IHitGeometry>(DielectricHit h,float3 direction,float2 pixel,Lobe lobe) {
    if(h.field) return field_surface<V,P>(h.object,h.field_hit,direction,pixel,lobe);
    return hit_surface<V,P,G>(h.instance,h.primitive,h.object,h.world_to_object,h.barycentrics,direction,pixel,lobe);
}
void path_boundary_error(uint code, uint object) {
    InterlockedMax(shader_errors[0],code);InterlockedMin(shader_errors[1],object);
}
void path_cross<let N:uint>(inout PathOccupancy<N> state, PathBoundary boundary, bool entering) {
    if (!state.cross(boundary,entering)) {path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,boundary.object);}
}
void plain_path_cross<let N:uint>(inout PathOccupancy<N> state,PathBoundary boundary,bool entering) {
    if(!state.cross<EqualBoundaryPriority>(boundary,entering)) path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,boundary.object);
}
// The solid a ray instance bounds, or 0xffffffff.
uint instance_solid<G:IHitGeometry>(uint instance,uint2 tables) {return G.word(tables.x+instance);}
// A solid's words (REG_PATH_SOLID_WORDS): object-space lo.xyz, hi.xyz, then
// the object row whose transform (the TLAS instance's) places them.
uint solid_words(uint solid,uint2 tables) {return tables.y+solid*REG_PATH_SOLID_WORDS;}
float3 solid_lo<G:IHitGeometry>(uint at) {return asfloat(uint3(G.word(at),G.word(at+1u),G.word(at+2u)));}
float3 solid_hi<G:IHitGeometry>(uint at) {return asfloat(uint3(G.word(at+3u),G.word(at+4u),G.word(at+5u)));}
float3x4 solid_world_to_object<G:IHitGeometry>(uint at) {return (float3x4)object_normal(G.word(at+6u));}
// Whether a solid's bounds, placed by world_to_object (camera containment has
// it from the ray query), can hold world, with the f32 error of that affine
// transform. passes::path::placed_bounds (and its test's copy of this margin)
// pads install bounds to hold every point this lets in. Unbounded solids
// (±inf) hold every point; a placement without an inverse (NaN) is no
// evidence against holding it.
bool solid_bounds_hold<G:IHitGeometry>(uint solid, float3x4 world_to_object, float3 world,uint2 tables) {
    let at=solid_words(solid,tables);
    let local=mul(world_to_object,float4(world,1.));
    let error=REG_FP32_GAMMA7*mul(abs(world_to_object),float4(abs(world),1.));
    return !any(local+error<solid_lo<G>(at)) && !any(local-error>solid_hi<G>(at));
}
// The tracked set can hold a solid the path left without crossing its
// boundary, where rounding loses an exit hit. A path is never inside a solid
// beyond its bounds, so at each hit, before its segment is attenuated or its
// interface resolved, drop the tracked solids whose bounds cannot hold the hit
// point (a hit on a solid's own surface is within them). The set keeps its
// order, so the owner among the others is unchanged, and it never exceeds the
// installed bound N: at most N solids' bounds hold one point
// (passes::path::occupancy; for solids moved by an animated node, only up to
// that node's world translation, which install does not bound).
void reconcile_occupancy<G:IHitGeometry,let N:uint>(inout PathOccupancy<N> state, float3 world, uint hit_solid,uint2 tables) {
    for (var slot=state.count;slot>0u;slot--) {
        let solid=state.at(slot-1u).object;
        if (solid!=hit_solid && !solid_bounds_hold<G>(solid,solid_world_to_object<G>(solid_words(solid,tables)),world,tables)) {state.remove(slot-1u);}
    }
}
// Occupancy describes the volume, independently of the stochastic surface lobe.
// A heterogeneous interface mixture with conflicting volumes is ambiguous;
// reject it instead of making camera-inside state depend on the selected lobe.
void validate_boundary<V:IVariant,P:IProgram>(Varying v,bool entering,Coordinates k,Surface s) {
    let c=s.closure;
    if (P.lobes && s.lobes>1u) {
        for (var i=0u;i<s.lobes;i++) {
            let other=surface_at<V,P>(v,entering,s.view,k,Lobe(i,0.)).closure;
            if (other.geometry_thin_walled!=c.geometry_thin_walled || other.specular_ior!=c.specular_ior || other.transmission_dispersion_scale!=c.transmission_dispersion_scale || other.transmission_dispersion_abbe_number!=c.transmission_dispersion_abbe_number || any(other.transmission_color!=c.transmission_color) || other.transmission_depth!=c.transmission_depth) {path_boundary_error(REG_PATH_BOUNDARY_AMBIGUOUS,v.object);}
        }
    }
}
PathBoundary boundary_descriptor<V:IVariant,P:IProgram,G:IHitGeometry>(DielectricHit hit, float3 direction, HitSurface h,uint2 tables,float2 pixel) {
    validate_boundary<V,P>(h.v,h.entering,h.k,h.s);
    return path_boundary(instance_solid<G>(hit.instance,tables),h.s.closure,G.word(hit.instance*REG_RAY_SLOT_WORDS+1u));
}
// A transaction in the existing set: retain exited descriptors until the BSDF
// decides, and provisionally insert entries. Their union fits the installed N
// because every old and acquired solid's bounds hold this same point.
struct BoundaryEvent<let N:uint> {
    // A solid addresses seven words in a u32-indexed table, leaving two tag bits.
    uint tagged[N>0u ? N : 1u];uint count;
    uint object_at(uint i) {return tagged[i]&0x3fffffffu;}
    uint kind_at(uint i) {return tagged[i]>>30u;}
    uint change(uint object) {
        for(uint i=0u;i<count;i++) {if(object_at(i)==object) return kind_at(i);}
        return 0u;
    }
    bool contains(uint object) {
        for(uint i=0u;i<count;i++) {if(object_at(i)==object) return true;}
        return false;
    }
    [mutating] void add(inout PathOccupancy<N> state,PathBoundary b,bool entering) {
        let slot=state.find(b.object);
        let kind=entering ? (slot==state.count ? 2u : 0u) : (slot<state.count ? 1u : 0u);
        tagged[count++]=b.object|(kind<<30u);
        if(entering && slot==state.count && !state.cross(b,true)) {
            tagged[count-1u]&=0x3fffffffu;path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,b.object);
        }
    }
    void order_entries(inout PathOccupancy<N> state) {
        if(count<=1u) return;
        for(uint i=1u;i<state.count;i++) {
            let b=state.at(i);if(change(b.object)!=2u) continue;
            var slot=i;
            while(slot>0u) {
                let previous=state.at(slot-1u);
                if(boundary_priority(previous)!=boundary_priority(b) || previous.object<b.object || change(previous.object)!=2u) break;
                state.set(slot,previous);slot--;
            }
            state.set(slot,b);
        }
    }
    PathBoundary owner(PathOccupancy<N> state) {
        for(var slot=state.count;slot>0u;slot--) {
            let b=state.at(slot-1u);if(change(b.object)!=1u) return b;
        }
        return path_air();
    }
    void finish(inout PathOccupancy<N> state,bool transmitted) {
        for(var slot=state.count;slot>0u;slot--) {
            if(change(state.at(slot-1u).object)==(transmitted ? 1u : 2u)) state.remove(slot-1u);
        }
    }
};
// Resume within the existing hit-position uncertainty, excluding processed
// primitives. Native intersections of a shared face can differ by many t ULPs
// (large triangles, oblique rays); never offset through that interval.
// Every gathered solid must hold the same hit point under the existing bounds
// test, preserving the installed occupancy bound and rejecting separated boxes.
// Deduplicate solids and order simultaneous entries
// by solid identity so last-entered ties do not depend on hardware traversal.
float boundary_distance_error(float3 error,float3 face,float3 direction) {
    return dot(abs(face),error)/max(abs(dot(face,direction)),REG_FP32_GAMMA7);
}
// Only reject traversal outside conservative static surface bounds. Closure
// uncertainty can make a medium unbounded without making its surface infinite.
// Moving/unknown geometry and larger sets always retain the normal AS query.
bool boundary_contact_possible<G:IHitGeometry>(uint solid,float3 world,float3 interval,uint2 tables) {
    let count=G.word(tables.y-1u);
    if(count>8u) return true;
    for(uint i=0u;i<count;i++) {
        if(i==solid) continue;
        let at=tables.y+count*REG_PATH_SOLID_WORDS+i*6u;
        if(!any(world+interval<solid_lo<G>(at)) && !any(world-interval>solid_hi<G>(at))) return true;
    }
    return false;
}
BoundaryEvent<N> boundary_event<V:IVariant,P:IProgram,G:IHitGeometry,let N:uint>(
    RaytracingAccelerationStructure scene,float3 origin,float3 direction,DielectricHit first,float3 world,float3 error,float3 face,
    PathBoundary boundary,bool entering,inout PathOccupancy<N> state,uint2 tables,float2 pixel,uint mask VORK_WORK_PARAMETER) {
    BoundaryEvent<N> event={};
    if(N==0u) {path_cross(state,boundary,entering);return event;}
    event.add(state,boundary,entering);
    if(N<=1u || first.t<0.) return event;
    let distance_error=boundary_distance_error(error,face,direction);
    if(!boundary_contact_possible<G>(boundary.object,world,error+abs(direction)*distance_error,tables)) return event;
    let minimum=max(0.,first.t-distance_error);
    let maximum=min(REG_FP32_MAX,first.t+distance_error);
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=minimum;
    ray.TMax=asfloat(min(asuint(REG_FP32_MAX),asuint(maximum)+1u));
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(scene,RAY_FLAG_FORCE_NON_OPAQUE,mask,ray);
    while(query.Proceed()) {
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_work_reserve(medium_work,1u,7u,0u)) {break;}
#endif
        if(query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE || query.CandidateTriangleRayT()<minimum || query.CandidateTriangleRayT()>maximum) continue;
        let instance=query.CandidateInstanceIndex();let primitive=query.CandidatePrimitiveIndex();
        if(instance==first.instance && primitive==first.primitive) continue;
        let identity=instance_solid<G>(instance,tables);
        if(identity==0xffffffffu || !catcher_keeps(query.CandidateInstanceID())) continue;
        if(event.contains(identity) || !solid_bounds_hold<G>(identity,query.CandidateWorldToObject3x4(),world,tables)) continue;
        DielectricHit hit={};hit.t=first.t;hit.instance=instance;hit.primitive=primitive;hit.object=query.CandidateInstanceID();
        hit.barycentrics=query.CandidateTriangleBarycentrics();hit.world_to_object=query.CandidateWorldToObject3x4();
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_alpha_reserve(medium_work,2u,hit.object)) {break;}
#endif
        let h=dielectric_surface<V,P,G>(hit,direction,pixel,Lobe(0u,0.));
        if(h.s.discarded || h.s.closure.geometry_opacity<=0. || h.s.closure.geometry_thin_walled || h.s.closure.transmission_weight<=0.) continue;
        if(event.count>=N) {path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,hit.object);break;}
        let b=boundary_descriptor<V,P,G>(hit,direction,h,tables,pixel);
        event.add(state,b,h.entering);
    }
    event.order_entries(state);
    return event;
}
PathOccupancy<N> camera_occupancy<V:IVariant,P:IProgram,G:IHitGeometry,let N:uint>(RaytracingAccelerationStructure scene,float3 origin, float3 direction,uint2 tables,float2 pixel,uint field_offset,uint field_count VORK_WORK_PARAMETER) {
    PathOccupancy<N> state={};
    if (N==0u) {return state;}
    // Collect the nearest boundary per solid object in one shared-AS query.
    // Ordinary surface visibility is not the definition of medium occupancy.
    // Only a solid whose bounds hold the origin can contain it, and at most N
    // solids' bounds overlap any point at one time (passes::path::occupancy), so
    // N entries hold every candidate.
    uint seen[N>0u ? N : 1u]={};float nearest[N>0u ? N : 1u]={};bool inside[N>0u ? N : 1u]={};PathBoundary records[N>0u ? N : 1u]={};
    var seen_count=0u;
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=REG_FP32_MAX;
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(scene,RAY_FLAG_FORCE_NON_OPAQUE,255u,ray);
    while (query.Proceed()) {
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_work_reserve(medium_work,1u,7u,0u)) {break;}
#endif
        if (query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE) {continue;}
        if (!catcher_keeps(query.CandidateInstanceID())) {continue;}
        let instance=query.CandidateInstanceIndex();
        let identity=instance_solid<G>(instance,tables);
        if (identity==0xffffffffu) {continue;}
        // This instance's current transform places the solid's bounds.
        if (!solid_bounds_hold<G>(identity,query.CandidateWorldToObject3x4(),origin,tables)) {continue;}
        let object=query.CandidateInstanceID();let distance=query.CandidateTriangleRayT();
        var slot=0u;while (slot<seen_count && seen[slot]!=identity) {slot++;}
        if (slot<seen_count && distance>=nearest[slot]) {continue;}
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_alpha_reserve(medium_work,2u,object)) {break;}
#endif
        let h=hit_surface<V,P,G>(instance,query.CandidatePrimitiveIndex(),object,query.CandidateWorldToObject3x4(),query.CandidateTriangleBarycentrics(),direction,pixel,Lobe(0u,0.));
        let c=h.s.closure;
        if (slot==seen_count) {
            if (seen_count>=N) {path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,object);break;}
            seen[seen_count++]=identity;
        }
        nearest[slot]=distance;inside[slot]=!h.entering && !c.geometry_thin_walled && c.transmission_weight>0.;
        DielectricHit boundary_hit={};boundary_hit.instance=instance;boundary_hit.object=object;
        boundary_hit.primitive=query.CandidatePrimitiveIndex();boundary_hit.barycentrics=query.CandidateTriangleBarycentrics();boundary_hit.world_to_object=query.CandidateWorldToObject3x4();
        records[slot]=boundary_descriptor<V,P,G>(boundary_hit,direction,h,tables,pixel);
    }
    for(uint i=0u;i<field_count;i++) {
        let at=field_offset+i*3u;let instance=G.word(at);let object=G.word(at+1u);
        let identity=instance_solid<G>(instance,tables);
        if(!catcher_keeps(object) || (objects[object].metadata.y&~REG_OBJECT_IMPLICIT)==0u || collapsed_endpoint(objects[object].model) || identity==0xffffffffu || !solid_bounds_hold<G>(identity,(float3x4)object_normal(object),origin,tables)) continue;
        let root=field_intersection<G>(G.word(at+2u),object,origin,direction,0.,REG_FP32_MAX);
        if(!root.found) continue;
        let h=field_surface<V,P>(object,root,direction,pixel,Lobe(0u,0.));
        if(h.entering || h.s.closure.geometry_thin_walled || h.s.closure.transmission_weight<=0.) continue;
        if(seen_count>=N) {path_boundary_error(REG_PATH_BOUNDARY_OVERFLOW,object);break;}
        let slot=seen_count++;seen[slot]=identity;nearest[slot]=root.t;inside[slot]=true;
        DielectricHit boundary_hit={};boundary_hit.instance=instance;boundary_hit.object=object;
        records[slot]=boundary_descriptor<V,P,G>(boundary_hit,direction,h,tables,pixel);
    }
    // Sort by priority, then outer to inner (nearest containing exit last). The tie key
    // is object identity, independent of acceleration-structure query order.
    float exits[N>0u ? N : 1u]={};
    for (var i=0u;i<seen_count;i++) {
        if (!inside[i]) {continue;}
        var slot=state.count;
        while (slot>0u && (boundary_priority(state.active[slot-1u])>boundary_priority(records[i]) || (boundary_priority(state.active[slot-1u])==boundary_priority(records[i]) && (exits[slot-1u]<nearest[i] || (exits[slot-1u]==nearest[i] && state.active[slot-1u].object>records[i].object))))) {
            state.active[slot]=state.active[slot-1u];exits[slot]=exits[slot-1u];slot--;
        }
        state.active[slot]=records[i];exits[slot]=nearest[i];state.count++;
    }
    return state;
}
