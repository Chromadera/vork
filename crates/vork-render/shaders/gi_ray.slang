#ifdef VORK_DIFFUSE_MEDIUM
import diffuse_medium;
// Preparation is retained with the incident accumulator for this exposure.
struct GiMediumQuery {
    float4 accumulation;
    float4 position_depth;
    float4 normal_view;
    float4 geometric_active;
    float4 error;
};
#ifdef VORK_METAL
[[vk::binding(1,3)]] RWStructuredBuffer<GiMediumQuery> gi_medium_accumulation : register(VORK_GI_MEDIUM_REGISTER);
#else
[[vk::binding(1,3)]] RWStructuredBuffer<GiMediumQuery> gi_medium_accumulation;
#endif
#endif
// Ray diffuse indirect shades off-screen hits through the raster surface
// evaluation (hit.slang) and the shared BSDF evaluator.
import vork_constants;
import openpbr_core;
import frame;
import vork_ray_sampling;
import vork_sampling;
import vork_ray_alpha;
import openpbr_inputs;
import bsdf_response;
import closure;
import variant;
import material;
import surface;
import hit;
import dielectric_scene;
import ray_coverage;
import hit_lighting;
import emitter_mesh;
import emitter_implicit;
import implicit_scene;
import surface_lighting;
import subsurface_transport;
#ifdef VORK_PROBES
import probes;
#endif
// rt::gi_entries: the scene globals (frame.slang) occupy bindings 0-9.
[[vk::binding(10,0)]] Texture2D<float4> receivers : register(t4);
[[vk::binding(11,0)]] Texture2D<float4> normals : register(t5);
[[vk::binding(12,0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> output : register(u6);
[[vk::binding(13,0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> incident_output : register(u7);
[[vk::binding(14,0)]] RaytracingAccelerationStructure ray_scene : register(t5);
[[vk::binding(15,0)]] StructuredBuffer<uint> ray_alpha : register(t6);
[[vk::binding(16,0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> cell_receivers : register(u8);
#ifdef VORK_DIFFUSE_MEDIUM
struct GiSources {uint4 topology;uint4 batch;};
[[vk::binding(17,0)]] ConstantBuffer<GiSources> gi_source_data : register(b7);
#define gi_sources gi_source_data.topology
#else
[[vk::binding(17,0)]] ConstantBuffer<uint4> gi_sources : register(b7);
#endif
[[vk::binding(18,0)]] StructuredBuffer<float> gi_emitter_weights : register(t8);

// Raster geometry buffers ray hits read in place (rt::hit_record, rt::hits_layout).
#ifdef VORK_METAL
[[vk::binding(0,3)]] ParameterBlock<RayHitBuffersTable> ray_hit_buffers : register(VORK_HIT_REGISTER);
#else
[[vk::binding(0,3)]] StructuredBuffer<uint> ray_hit_buffers[REG_RAY_HIT_BUFFERS];
#endif
uint ray_hit_buffers_load(uint buffer,uint index) {
#ifdef VORK_METAL
    return ray_hit_buffers.buffers[buffer][index];
#else
    return ray_hit_buffers[NonUniformResourceIndex(buffer)][index];
#endif
}
struct GiGeometry : IHitGeometry {
    static uint word(uint index) {return ray_alpha[index];}
    static uint load(uint buffer, uint index) {return ray_hit_buffers_load(buffer,index);}
};
#ifdef VORK_DIFFUSE_MEDIUM
float3 gi_medium_visibility<P:IProgram>(float3 position,uint index) {
    return diffuse_source_visibility<P,GiGeometry>(ray_scene,ray_alpha,position,index);
}
#ifdef VORK_MEDIUM_MESH_SOURCES
float3 gi_medium_mesh<P:IProgram>(float3 position,float3 ray,float g) {
    return diffuse_mesh_illumination<P,GiGeometry>(ray_scene,ray_alpha,gi_emitter_weights,position,ray,g);
}
#endif
#endif
export float3 ray_eye() {return frame.eye.xyz;}
uint2 image_size(Texture2D<float4> t) {uint w,h;t.GetDimensions(w,h);return uint2(w,h);}
uint2 image_size(RWTexture2D<float4> t) {uint w,h;t.GetDimensions(w,h);return uint2(w,h);}


float gi_mis(float sampled,float other) {
    if(other<=0.) {return 1.;}if(sampled<=0.) {return 0.;}
    let ratio=other/sampled;return 1./(1.+ratio*ratio);
}
float gi_emitter_pdf<P:IProgram>(RayIntersection hit,float3x4 inverse,float3 from) {
    let base=gi_sources.x;
    if(ray_alpha[base]==0u) {return 0.;}
    let probability=emitter_mesh_probability<GiGeometry>(base,hit.instance_index,hit.primitive_index,gi_emitter_weights);
    if(probability<=0.) {return 0.;}
    let geometry=hit_emitter<GiGeometry>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,inverse,hit.barycentrics);
    return probability*emitter_mesh_pdf<GiGeometry>(base,hit.instance_index,geometry.area,from,geometry.point,geometry.face);
}
// A physical source connection complements the cosine continuation. Only source
// emission receives MIS; reflected radiance retains the hemisphere estimator.
float3 gi_source<V:IVariant,P:IProgram>(float3 from,float3 normal,float3 geometric,float3 error,uint2 pixel,uint index,uint count) {
    let base=gi_sources.x;if(ray_alpha[base]==0u) {return float3(0.);}
    let uv=sample_2d(pixel,index,count,REG_SAMPLING_STREAM_PROBE_EMITTER);
    let extra=sample_2d(pixel,index,count,REG_SAMPLING_STREAM_PROBE_TRIANGLE);
    let selected=emitter_mesh_sample<GiGeometry>(base,float3(extra.x,uv),gi_emitter_weights);
    if((objects[selected.object].metadata.y&REG_RAY_DIFFUSE)==0u) {return float3(0.);}
    let inverse=(float3x4)object_normal(selected.object);
    let geometry=hit_emitter<GiGeometry>(selected.instance,selected.primitive,selected.object,inverse,selected.barycentrics);
    let delta=geometry.point-from;let distance=length(delta);if(distance<=0.) {return float3(0.);}
    let direction=delta/distance;let cosine=max(dot(normal,direction),0.);
    if(cosine<=0. || dot(geometric,direction)<0.) {return float3(0.);}
    let hit=RayIntersection(selected.instance,selected.object,selected.primitive,selected.barycentrics);
    let pdf=gi_emitter_pdf<P>(hit,inverse,from);if(pdf<=0.) {return float3(0.);}
    if(!diffuse_candidate_accepts<P,GiGeometry>(ray_alpha,hit,dot(geometry.face,-direction)>0.,inverse,direction,float2(0.))) {return float3(0.);}
    let source=hit_surface<V,P,GiGeometry>(selected.instance,selected.primitive,selected.object,inverse,selected.barycentrics,direction,float2(0.));
    if(source.s.discarded) {return float3(0.);}
    let emitted=emitter_mesh_radiance<V,P>(source);if(!any(emitted!=float3(0.))) {return float3(0.);}
    let start=ray_origin(from,geometric,direction,error);
    let end=surface_ray_origin(geometry.point,surface_position_error(selected.object,geometry.point),source.face,-direction);
    let connection=end-start;let extent=length(connection);if(extent<=0.) {return float3(0.);}
    RayDesc ray;ray.Origin=start;ray.Direction=connection/extent;ray.TMin=0.;ray.TMax=extent;
    if(diffuse_ray_occluded<P,GiGeometry>(ray_scene,ray_alpha,ray,REG_RAY_GI_OCCLUDER,float2(0.))) {return float3(0.);}
    let continuation=cosine/REG_PI;var continuation_pdf=continuation;
#ifdef VORK_DIFFUSE_MEDIUM
    continuation_pdf=medium_source_pdf(from,normal,direction);
#endif
    var radiance=emitted;
#ifdef VORK_DIFFUSE_MEDIUM
    radiance=diffuse_medium_radiance(start,ray.Direction,extent,radiance,extra.y,false);
#endif
    return radiance*(continuation/pdf)*gi_mis(pdf,continuation_pdf)*frame.radiance.w;
}

// The domain cone proposes a direction; only the exact field root emits.
uint4 gi_fields() {return uint4(frame.field_geometry.xy,gi_sources.yz);}
float3 gi_field_source<V:IVariant,P:IProgram>(float3 from,float3 normal,float3 geometric,float3 error,uint2 pixel,uint index,uint count) {
    let table=gi_fields();if(table.w==0u) return float3(0.);
    let uv=sample_2d(pixel,index,count,REG_SAMPLING_STREAM_PROBE_EMITTER);
    let extra=sample_2d(pixel,index,count,REG_SAMPLING_STREAM_PROBE_TRIANGLE);
    let selected=field_emitter<GiGeometry>(table,min(uint(extra.x*float(table.w)),table.w-1u));
    if((objects[selected.object].metadata.y&REG_RAY_DIFFUSE)==0u || collapsed_endpoint(objects[selected.object].model)) return float3(0.);
    float density;let direction=field_emitter_direction<GiGeometry>(selected,from,uv,density);
    let cosine=max(dot(normal,direction),0.);let pdf=density/float(table.w);
    if(pdf<=0. || cosine<=0. || dot(geometric,direction)<0.) return float3(0.);
    var minimum=0.;
    for(;;) {
        let root=field_intersection<GiGeometry>(selected.descriptor,selected.object,from,direction,minimum,REG_FP32_MAX);
        if(!root.found) return float3(0.);
        let source=field_surface<V,P>(selected.object,root,direction,float2(0.),Lobe(0u,0.));
        if(source.s.discarded || source.s.closure.geometry_opacity<=0.) {
            minimum=root.t+REG_FP32_GAMMA7*max(1.,abs(root.t));continue;
        }
        if(!material_covers_side<V>(source.v,source.entering)) return float3(0.);
        let emitted=emitter_mesh_radiance<V,P>(source)*source.s.closure.geometry_opacity;
        if(!any(emitted>float3(0.))) return float3(0.);
        let start=ray_origin(from,geometric,direction,error);
        let end=surface_ray_origin(source.v.world,surface_position_error(selected.object,source.v.world),source.face,-direction);
        let delta=end-start;let extent=length(delta);if(extent<=0.) return float3(0.);
        RayDesc ray;ray.Origin=start;ray.Direction=delta/extent;ray.TMin=0.;ray.TMax=extent;
        if(diffuse_ray_occluded<P,GiGeometry>(ray_scene,ray_alpha,ray,REG_RAY_GI_OCCLUDER,float2(0.))) return float3(0.);
        var continuation=cosine/REG_PI;var radiance=emitted;
#ifdef VORK_DIFFUSE_MEDIUM
        continuation=medium_source_pdf(from,normal,direction);
        radiance=diffuse_medium_radiance(start,ray.Direction,extent,radiance,extra.y,false);
#endif
        return radiance*(cosine/(REG_PI*pdf))*gi_mis(pdf,continuation)*frame.radiance.w;
    }
}

// Shared surface shadows at GI hits: one ray toward each light (hit_lighting).
struct GiShadows<P:IProgram> : IShadows {
    static const bool probe_rays=true;
    static bool probe_visible(float3 world,float3 normal,float3 position_error,float3 probe) {return probe_connection_visible<P,GiGeometry>(ray_scene,ray_alpha,world,normal,position_error,probe);}
    static uint integration_count(uint index,out float3 normalization) {let count=index==0xffffffffu ? sample_count() : 1u;normalization=float3(float(count));return count;}
    static float3 integration_sample(uint tap,inout uint index,inout float3 world) {if(index==0xffffffffu) {index=tap;}return float3(1.);}
    static uint sample_count() {return uint(frame.temporal.y)*max(frame.sampling.w,1u)*uint(frame.ao.z);}
    static float3 visibility_direction(float3 world,float3 normal,LightValues record,uint light,float3 direction,float distance,float3 position_error,uint index,uint count,bool fibre=false) {return hit_shadow_direction<P,GiGeometry>(ray_scene,ray_alpha,light,world,normal,position_error,direction,distance,index,count,false);}
    static const bool subsurface=true;
    static float thickness(uint object,float3 world,float3 direction) {return subsurface_exit(ray_scene,object,world,direction);}
};
// Outgoing diffuse radiance at a hit the camera cannot see (hit_lighting).
float3 hit_shading<V:IVariant,P:IProgram>(RayIntersection hit,float3x4 world_to_object,float3 world,float3 direction,float3 from,float continuation_pdf,uint2 pixel,uint index) {
    if (ray_alpha[hit.instance_index*REG_RAY_SLOT_WORDS+2u]==REG_RAY_NO_ALPHA) {return float3(0.);}
    // Hits filter textures with the camera pixel's footprint at the hit.
    // A mixed surface (Blend programs) contributes one lobe picked per ray.
    let u=select(P.lobes,float(screen_ray_hash(pixel,index,REG_SAMPLING_STREAM_RAY_GI_LOBE))*REG_RADICAL_INVERSE_SCALE,0.);
    var h=hit_surface<V,P,GiGeometry>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,world_to_object,hit.barycentrics,direction,float2(2.)/float2(image_size(receivers)),Lobe(select(P.lobes,LOBE_PICK,0u),u));
    float3 emitted;
    let radiance=finalized_hit_radiance<V,GiShadows<P>,P>(h,pixel,index,REG_RAY_DIFFUSE,emitted);
    let source_pdf=gi_emitter_pdf<P>(hit,world_to_object,from);
    return (radiance-emitted*(1.-gi_mis(continuation_pdf,source_pdf)))*frame.radiance.w;
}
// Resolve consumes only the weighted ambient baseline and floating-point diffuse response.
// Include the complete separable blur and upsampling support before discarding
// a query. Keep its geometry guides so every retained filter weight is unchanged.
[[vk::binding(19,0)]] Texture2D<float4> demand_ambient : register(t9);
[[vk::binding(20,0)]] StructuredBuffer<float> demand_response : register(t9);
bool gi_consumed(uint2 cell) {
    let divisor=int(frame.ao.w);
    let radius=REG_AO_BLUR_RADIUS+REG_AO_UPSAMPLE_RADIUS;
    let lo=max((int2(cell)-radius)*divisor,int2(0));
    let hi=min((int2(cell)+radius+1)*divisor,int2(image_size(receivers)));
    for(int y=lo.y;y<hi.y;y++) {for(int x=lo.x;x<hi.x;x++) {
        let p=int3(x,y,0);
        if(receivers.Load(p).w>=1.) continue;
        let at=(uint(y)*image_size(receivers).x+uint(x))*3u;
        if(any(demand_ambient.Load(p).rgb!=float3(0.)) ||
            (frame.flags.w>0. && any(float3(demand_response[at],demand_response[at+1u],demand_response[at+2u])!=float3(0.)))) return true;
    }}
    return false;
}
// Scenes with programs compile their own entry with their Programs (SlangForward::program_kernel).
void gi_pixel<V:IVariant,P:IProgram>(uint3 gid) {
    if (any(gid.xy>=image_size(output))) {return;}
#ifdef VORK_DIFFUSE_MEDIUM
    let at=gid.y*image_size(output).x+gid.x;
    let begin=gi_source_data.batch.x;
    GiMediumQuery prepared;
    if(begin==0u) {
#endif
    let surface=ao_surface(receivers,normals,gid.xy,int(frame.ao.w),frame.view);
    if(surface.depth<1.) {cell_receivers[gid.xy]=float4(surface.position,surface.depth);}
    let active=surface.depth<1. && gi_consumed(gid.xy);
    let ng=active ? screen_geometric_normal(receivers,surface.pixel,surface.position,surface.normal) : surface.normal;
    let position_error=active ? surface_position_error(surface.object,surface.position) : float3(0.);
#ifdef VORK_DIFFUSE_MEDIUM
    prepared.accumulation=float4(0.);
    prepared.position_depth=float4(surface.position,surface.depth);
    prepared.normal_view=float4(surface.normal,surface.view_depth);
    prepared.geometric_active=float4(ng,active ? 1. : 0.);
    prepared.error=float4(position_error,0.);
    gi_medium_accumulation[at]=prepared;
#endif
    if(!active) {
        output[gid.xy]=surface.depth>=1. ? float4(1.,0.,0.,0.) : float4(1.,surface.view_depth,encode_octahedron(surface.normal));
        incident_output[gid.xy]=float4(0.);return;
    }
#ifdef VORK_DIFFUSE_MEDIUM
    } else {prepared=gi_medium_accumulation[at];}
    // Inactive cells skip every later batch before receiver reconstruction or
    // the support-aware demand scan. Their first batch published filter guides.
    if(prepared.geometric_active.w==0.) {return;}
    let position=prepared.position_depth.xyz;let n=prepared.normal_view.xyz;
    let view_depth=prepared.normal_view.w;
    let ng=prepared.geometric_active.xyz;let position_error=prepared.error.xyz;
#else
    let position=surface.position;let n=surface.normal;let view_depth=surface.view_depth;
    let begin=0u;
#endif
    let rays=uint(frame.ao.z)*max(frame.sampling.w,1u);let count=rays*uint(max(frame.temporal.y,1.));let first=uint(frame.temporal.x)*rays;
    let basis=ray_basis(n);
    // Replacement and radiance estimate the same clipped cosine measure.
    // The signed residual may be negative at finite sample counts.
    var hits=0.;var incident=float3(0.);
#ifdef VORK_DIFFUSE_MEDIUM
    let end=min(begin+REG_MEDIUM_GI_RAY_BATCH,rays);
    incident=prepared.accumulation.xyz;hits=prepared.accumulation.w;
#else
    let end=rays;
#endif
    for(var k=begin;k<end;k++) {
        incident+=gi_source<V,P>(position,n,ng,position_error,gid.xy,first+k,count);
        incident+=gi_field_source<V,P>(position,n,ng,position_error,gid.xy,first+k,count);
        let disk=ray_concentric_disk(sample_2d(gid.xy,first+k,count,REG_SAMPLING_STREAM_RAY_GI));
        var direction=mul(float3(disk,sqrt(max(1.-dot(disk,disk),0.))),basis);
        var direction_weight=1.;
#ifdef VORK_DIFFUSE_MEDIUM
        direction_weight=medium_source_direction(position,n,gid.xy,first+k,count,direction);
#endif
        // Directions below the geometric plane would hit the receiver itself: misses.
        // Tangent ones (the sample square's edge maps to the horizon) are traced.
        if (dot(direction,ng)<0.) {continue;}
        let origin=ray_origin(position,ng,direction,position_error);
        RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=REG_FP32_MAX;
        RayQuery<RAY_FLAG_NONE> query;
        query.TraceRayInline(ray_scene,RAY_FLAG_FORCE_NON_OPAQUE,REG_RAY_GI_OCCLUDER,ray);
        while (query.Proceed()) {
            let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
            if (diffuse_candidate_accepts<P,GiGeometry>(ray_alpha,candidate,query.CandidateTriangleFrontFace(),query.CandidateWorldToObject3x4(),direction,float2(2.)/float2(image_size(receivers)))) {query.CommitNonOpaqueTriangleHit();}
        }
        DielectricHit closest={};closest.t=-1.;
        if(query.CommittedStatus()==COMMITTED_TRIANGLE_HIT) {
            closest.t=query.CommittedRayT();closest.instance=query.CommittedInstanceIndex();closest.object=query.CommittedInstanceID();
            closest.primitive=query.CommittedPrimitiveIndex();closest.barycentrics=query.CommittedTriangleBarycentrics();closest.world_to_object=query.CommittedWorldToObject3x4();
        }
        closest=merge_field_hits<P,GiGeometry>(closest,ray_fields(ray_alpha),origin,direction,ray.TMax,REG_RAY_GI_OCCLUDER);
        if (closest.t<0.) {
#if defined(VORK_PROBES) || defined(VORK_DIFFUSE_MEDIUM)
            // Integrate the complete escaping segment, including medium sources.
            hits+=direction_weight;var radiance=world_radiance(direction);
#ifdef VORK_DIFFUSE_MEDIUM
            radiance=diffuse_medium_radiance(origin,direction,REG_FP32_MAX,radiance,sample_2d(gid.xy,first+k,count,REG_SAMPLING_STREAM_VOLUME).x);
#endif
            incident+=radiance*frame.radiance.w*direction_weight;
#endif
            continue;
        }
        if(closest.field) {
            let h=dielectric_surface<V,P,GiGeometry>(closest,direction,float2(2.)/float2(image_size(receivers)),Lobe(0u,0.));
            hits+=1.;
            float3 emitted;
            var radiance=finalized_hit_radiance<V,GiShadows<P>,P>(h,gid.xy,first+k,REG_RAY_DIFFUSE,emitted);
            let continuation=direction_weight>0. ? max(dot(n,direction),0.)/(REG_PI*direction_weight) : 0.;
            radiance-=emitted*(1.-gi_mis(continuation,field_emitter_pdf<GiGeometry>(gi_fields(),closest.instance,position)));
#ifdef VORK_DIFFUSE_MEDIUM
            radiance=diffuse_medium_radiance(origin,direction,closest.t,radiance,sample_2d(gid.xy,first+k,count,REG_SAMPLING_STREAM_VOLUME).x);
#endif
            incident+=radiance*frame.radiance.w*direction_weight;
            continue;
        }
        let hit=RayIntersection(query.CommittedInstanceIndex(),query.CommittedInstanceID(),query.CommittedPrimitiveIndex(),query.CommittedTriangleBarycentrics());
        hits+=direction_weight;
        let world=origin+direction*query.CommittedRayT();
        // Evaluate outgoing diffuse radiance at the actual hit. Camera colour
        // contains a different view's glossy and transmission response.
        let cosine_pdf=max(dot(n,direction),0.)/REG_PI;
        let continuation_pdf=direction_weight>0. ? cosine_pdf/direction_weight : 0.;
        var radiance=hit_shading<V,P>(hit,query.CommittedWorldToObject3x4(),world,direction,position,continuation_pdf,gid.xy,first+k);
#ifdef VORK_DIFFUSE_MEDIUM
        radiance=diffuse_medium_radiance(origin,direction,query.CommittedRayT(),radiance/frame.radiance.w,sample_2d(gid.xy,first+k,count,REG_SAMPLING_STREAM_VOLUME).x)*frame.radiance.w;
#endif
        incident+=radiance*direction_weight;
    }
#ifdef VORK_DIFFUSE_MEDIUM
    if(end<rays) {gi_medium_accumulation[at].accumulation=float4(incident,hits);return;}
#endif
    let total=float(max(rays,1u));
    var estimate=incident/total;
    output[gid.xy]=float4(1.-hits/total,view_depth,encode_octahedron(n));
    incident_output[gid.xy]=float4(estimate,0.);
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void compute(uint3 gid : SV_DispatchThreadID) {gi_pixel<Unspecialized,NoPrograms>(gid);}
