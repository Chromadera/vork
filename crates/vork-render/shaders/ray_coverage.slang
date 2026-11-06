// Candidate coverage shares the hit-to-surface evaluator with committed hits.
// P.coverage erases geometry reads and program dispatch in builtin-only scenes.
import vork_constants;
import frame;
import vork_material_table;
import vork_shadow_coverage;
import vork_ray_alpha;
import hit;
import implicit_scene;
import surface;
import variant;
import bsdf;
import light_connection;
import vork_ray_sampling;
public float candidate_coverage<P:IProgram,G:IHitGeometry>(StructuredBuffer<uint> words, RayIntersection hit, float3x4 world_to_object, float3 direction, float2 pixel) {
    let r=objects[hit.instance_custom_data].material_index;
    let record=material_records[r];
    if (P.coverage && record.tag.x!=0u && record.extra.y>0.) {
        // Candidate masks sample a point, like the table path below. Keep
        // geometric derivatives for normal frames; only image LOD is zero.
        let texture_scale=material_texture_scale;material_texture_scale=0.;
        let h=hit_surface<Unspecialized,P,G>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,world_to_object,hit.barycentrics,direction,pixel);
        material_texture_scale=texture_scale;
        return candidate_scalar_opacity(r,h.s.closure.geometry_opacity);
    }
    return ray_candidate_opacity(words,hit,r);
}
public bool candidate_accepts<P:IProgram,G:IHitGeometry>(StructuredBuffer<uint> words, RayIntersection hit, float3x4 world_to_object, float3 direction, float2 pixel) {
    return candidate_coverage<P,G>(words,hit,world_to_object,direction,pixel)>0.;
}
// Reflection rays integrate blended surface coverage. Keep the sequence fixed as camera samples
// move their hit point, so coverage remains stratified over the exposure.
public bool reflection_candidate_accepts<P:IProgram,G:IHitGeometry>(StructuredBuffer<uint> words, RayIntersection hit, float3x4 world_to_object, float3 direction, float2 pixel, uint2 cell, uint index, uint count) {
    let opacity=candidate_coverage<P,G>(words,hit,world_to_object,direction,pixel);
    let seed=cell^uint2(pixel_hash(hit.primitive_index),pixel_hash(hit.instance_custom_data));
    return coverage_accepts(opacity,seed,index,count);
}


// All-candidate walks may report both triangles along a shared edge. A half-open edge
// owns that crossing once, so a sheet does not square its RGB transmission.
// Normalize projected winding before the top-left rule; separate coincident
// objects still each contribute their own crossing.
public bool connection_owns_hit<G:IHitGeometry>(RayIntersection hit,float3x4 inverse,float3 direction) {
    let bary=float3(1.-hit.barycentrics.x-hit.barycentrics.y,hit.barycentrics);
    if (all(bary!=float3(0.))) {return true;}
    let triangle=hit_triangle<G>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,inverse);
    let model=object_model(hit.instance_custom_data);
    let axis=abs(direction.x)>abs(direction.y) ? (abs(direction.x)>abs(direction.z) ? 0u : 2u) : (abs(direction.y)>abs(direction.z) ? 1u : 2u);
    float2 p[3];
    for(uint i=0u;i<3u;i++) {
        let world=mul(float4(triangle.c[i].POSITION,1.),model).xyz;
        p[i]=axis==0u ? world.yz : (axis==1u ? world.zx : world.xy);
    }
    let a=p[1]-p[0];let b=p[2]-p[0];let positive=a.x*b.y-a.y*b.x>0.;
    for(uint i=0u;i<3u;i++) {
        if(bary[i]!=0.) {continue;}
        let edge=(p[(i+2u)%3u]-p[(i+1u)%3u])*(positive ? 1. : -1.);
        if(!(edge.y>0. || (edge.y==0. && edge.x<0.))) {return false;}
    }
    return true;
}
// Exact straight-segment surface transmission. Coverage is scalar; a thin
// null interface contributes its RGB closure throughput. Nonabsorbing IOR-1
// solids also have a straight delta; refracting solids block this segment.
// Shared by every triangle backend.
public ConnectionVisibility connection_candidate_transmittance<P:IProgram,G:IHitGeometry>(StructuredBuffer<uint> words, RayIntersection hit, float3x4 inverse, float3 direction, float2 pixel, float3 world, uint sample_index, float incident_ior=1., bool null_sheets=true) {
    let opacity=candidate_coverage<P,G>(words,hit,inverse,direction,pixel);
    if (opacity<=0.) {return ConnectionVisibility(float3(1.),0u);}
    if (!null_sheets) {return ConnectionVisibility(float3(1.-opacity),0u);}
    let spatial=pixel_hash(asuint(world.x)^asuint(world.y)^asuint(world.z)^hit.instance_custom_data^hit.primitive_index);
    let u=float(screen_ray_hash(uint2(spatial,hit.instance_custom_data),sample_index,REG_SAMPLING_STREAM_SHADOW_ALPHA)>>8u)*(256.*REG_RADICAL_INVERSE_SCALE);
    let h=hit_surface<Unspecialized,P,G>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,inverse,hit.barycentrics,direction,pixel,Lobe(select(P.lobes,LOBE_PICK,0u),u));
    let sheet=h.s.is_hair ? float3(0.) : transport_null_transmittance(h.s.closure,-direction,incident_ior,h.s.geometric,h.face)*h.s.weight;
    return ConnectionVisibility((1.-opacity)+opacity*sheet,any(sheet>float3(0.)) ? 1u : 0u);
}

// Diffuse visibility follows raster material sidedness. Queries force opaque
// geometry through candidates too; otherwise hardware would accept its back faces.
// Ray-query facing is in object space, matching the raster's material_front.
public bool diffuse_candidate_accepts<P:IProgram,G:IHitGeometry>(StructuredBuffer<uint> words, RayIntersection hit, bool front, float3x4 world_to_object, float3 direction, float2 pixel) {
    if (!material_covers_side<Unspecialized>(hit.instance_custom_data,front)) {return false;}
    // Diffuse caches admit fully present points, including authored cutouts.
    // Fractional coverage is not an opaque GI occluder.
    return candidate_coverage<P,G>(words,hit,world_to_object,direction,pixel)>=1.;
}

// Shared any-hit traversal for diffuse occlusion and finite probe connections.
public bool diffuse_ray_occluded<P:IProgram,G:IHitGeometry>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,RayDesc ray,uint mask,float2 pixel) {
    let fields=ray_fields(words);
    for(uint i=0u;i<fields.y;i++) {
        let at=fields.x+i*3u;let object=G.word(at+1u);
        if((objects[object].metadata.y&mask)==0u || collapsed_endpoint(objects[object].model)) continue;
        var minimum=ray.TMin;
        for(;;) {
            let root=field_intersection<G>(G.word(at+2u),object,ray.Origin,ray.Direction,minimum,ray.TMax);
            if(!root.found) break;
            let h=field_surface<Unspecialized,P>(object,root,ray.Direction,pixel,Lobe(0u,0.));
            if(!h.s.discarded && h.s.closure.geometry_opacity>=1. && material_covers_side<Unspecialized>(h.v,h.entering)) return true;
            minimum=root.t+REG_FP32_GAMMA7*max(1.,abs(root.t));if(minimum>=ray.TMax) break;
        }
    }
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;
    query.TraceRayInline(scene,RAY_FLAG_FORCE_NON_OPAQUE,mask,ray);
    while (query.Proceed()) {
        let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        if (diffuse_candidate_accepts<P,G>(words,candidate,query.CandidateTriangleFrontFace(),query.CandidateWorldToObject3x4(),ray.Direction,pixel)) {query.CommitNonOpaqueTriangleHit();}
    }
    return query.CommittedStatus()!=COMMITTED_NOTHING;
}

// A finite diffuse connection uses the same accepted geometry as probe capture.
// Offset into the receiver's front side even when the probe lies behind it:
// offsetting toward that probe would step across a thin wall before tracing.
public bool probe_connection_visible<P:IProgram,G:IHitGeometry>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,float3 world,float3 normal,float3 position_error,float3 probe) {
    let origin=surface_ray_origin(world,position_error,normal,normal);
    let delta=probe-origin;let distance=length(delta);
    if (distance==0.) {return true;}
    let direction=delta/distance;
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=distance;
    return !diffuse_ray_occluded<P,G>(scene,words,ray,REG_RAY_GI_OCCLUDER,float2(0.));
}
