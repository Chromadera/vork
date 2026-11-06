#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
// Mesh-source incident radiance for the shared medium lighting integral.
// Geometry and surface emission remain in the existing scene ray/material tables.
import frame;
import vork_constants;
import surface;
import variant;
import material;
import vork_material_table;
import vork_ray_sampling;
#if VORK_VOLUME_SHADOWS
internal extern float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint);
#endif
import hit;
import emitter_mesh;
import ray_coverage;
import shadow_coverage;
import vork_medium;
import vork_sampling;
import vork_visibility;
import vork_ray_alpha;
public float3 medium_mesh_incident<P:IProgram,G:IHitGeometry>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,
    StructuredBuffer<float> weights,uint base,float3 position,float3 ray,float g,uint2 pixel,uint index,uint count) {
    let triangles=words[base+1u];if(triangles==0u) {return float3(0.);}
    var u=sample_2d(pixel,index,max(count,1u),REG_SAMPLING_STREAM_REFLECTION_EMITTER);
    let extra=sample_2d(pixel,index,max(count,1u),REG_SAMPLING_STREAM_PROBE_TRIANGLE);
    let selected=emitter_mesh_sample<G>(base,float3(extra.x,u),weights);
    if(selected.probability<=0.) {return float3(0.);}
    if((objects[selected.object].metadata.y&REG_RAY_VOLUME)==0u) {return float3(0.);}
    let inverse=(float3x4)object_normal(selected.object);
    let geometry=hit_emitter<G>(selected.instance,selected.primitive,selected.object,inverse,selected.barycentrics);
    let delta=geometry.point-position;let distance=length(delta);if(distance<=0.) {return float3(0.);}
    let direction=delta/distance;
    let pdf=emitter_mesh_pdf<G>(base,selected.instance,geometry.area,position,geometry.point,geometry.face)*selected.probability;
    if(pdf<=0.) {return float3(0.);}
    var visibility=float3(1.);
#if VORK_VOLUME_SHADOWS
    visibility=medium_transmittance(position,direction,medium_emitter(distance));
    if(!any(visibility>float3(0.))) {return float3(0.);}
#endif
    let source=hit_surface<Unspecialized,P,G>(selected.instance,selected.primitive,selected.object,inverse,selected.barycentrics,direction,float2(0.),Lobe(0u,0.));
    if(source.s.discarded || !material_covers_side<Unspecialized>(source.v,source.entering)) {return float3(0.);}
    let emitted=emitter_mesh_radiance<Unspecialized,P>(source)*source.s.closure.geometry_opacity;
    if(!any(emitted>float3(0.))) {return float3(0.);}
    let end=ray_origin(geometry.point,geometry.face,-direction,surface_position_error(selected.object,geometry.point));
    let connection=end-position;let length_to_source=length(connection);let axis=connection/length_to_source;
    RayDesc desc;desc.Origin=position;desc.Direction=axis;desc.TMin=0.;desc.TMax=length_to_source;
    RayQuery<RAY_FLAG_NONE> query;query.TraceRayInline(scene,RAY_FLAG_FORCE_NON_OPAQUE,REG_RAY_SHADOW,desc);
    while(query.Proceed()) {
        let hit=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        if(!connection_owns_hit<G>(hit,query.CandidateWorldToObject3x4(),axis)) {continue;}
        visibility*=shadow_candidate_transmittance<P,G>(scene,words,hit,query.CandidateWorldToObject3x4(),desc,query.CandidateTriangleRayT(),float2(0.),index);
        if(!any(visibility>float3(0.))) {return float3(0.);}
    }
    visibility*=shadow_contained_transmittance<P,G>(scene,words,desc,true,0u,float2(0.),index);
    return emitted*visibility*(medium_phase_weight(direction,ray,g)/pdf);
}
