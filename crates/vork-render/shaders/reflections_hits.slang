// Ray-traced reflections whose unseen hits shade their full surface (hit.slang,
// hit_lighting) on the shared Vulkan/Metal globals layout.
import vork_constants;
import frame;
import vork_ray_sampling;
import vork_ray_alpha;
import surface;
import variant;
import hit;
import dielectric_scene;
import emitter_mesh;
import vork_material_table;
import hit_lighting;
import vork_prelude;
import vork_sampling;
import vork_visibility;
import reflection_environment;
import reflection_receiver;
import reflection_bsdf;
import ray_coverage;
import surface_lighting;
import subsurface_transport;
// rt::reflections_hits_entries: the scene globals (frame.slang) occupy bindings 0-9.
[[vk::binding(10,0)]] Texture2D<float4> color : register(t4);
[[vk::binding(11,0)]] Texture2D<float4> receivers : register(t5);
[[vk::binding(12,0)]] Texture2D<float4> normals : register(t6);
[[vk::binding(13,0)]] Texture2D<float4> ambient : register(t7);
[[vk::binding(14,0)]] [[vk::image_format("rgba16f")]] RWTexture2DArray<float4> output : register(u8);
[[vk::binding(15,0)]] RaytracingAccelerationStructure ray_scene : register(t5);
[[vk::binding(16,0)]] StructuredBuffer<uint> ray_alpha : register(t6);
[[vk::binding(17,0)]] StructuredBuffer<uint> sampling : register(t7);
[[vk::binding(18,0)]] StructuredBuffer<uint> reflection_slots : register(t8);
[[vk::binding(19,0)]] StructuredBuffer<uint> reflection_guides : register(t9);
#ifdef VORK_METAL
struct ReflectionFields {StructuredBuffer<uint> buffers[REG_VISIBILITY_REFLECTION_WORDS];};
[[vk::binding(1,3)]] ParameterBlock<ReflectionFields> reflection_fields : register(VORK_REFLECTION_FIELDS_REGISTER);
#define reflection_records reflection_fields.buffers
#else
[[vk::binding(1,3)]] StructuredBuffer<uint> reflection_fields[REG_VISIBILITY_REFLECTION_WORDS];
#define reflection_records reflection_fields
#endif
[[vk::binding(20,0)]] ConstantBuffer<uint4> reflection_range : register(b10);
[[vk::binding(21,0)]] RWStructuredBuffer<float4> reflection_accumulation : register(u11);
[[vk::binding(22,0)]] StructuredBuffer<float> reflection_emitter_weights : register(t12);
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
export float3 ray_eye() {return frame.eye.xyz;}
struct ReflectionGeometry : IHitGeometry {
    static uint word(uint index) {return ray_alpha[index];}
    static uint load(uint buffer, uint index) {return ray_hit_buffers_load(buffer,index);}
};
// Sample the physical emitter and its visibility together at reflection hits.
struct ReflectionShadows<P:IProgram> : IShadows {
    static const bool probe_rays=true;
    static bool probe_visible(float3 world,float3 normal,float3 position_error,float3 probe) {return probe_connection_visible<P,ReflectionGeometry>(ray_scene,ray_alpha,world,normal,position_error,probe);}
    static uint integration_count(uint index,out float3 normalization) {let count=index==0xffffffffu ? sample_count() : 1u;normalization=float3(float(count));return count;}
    static float3 integration_sample(uint tap,inout uint index,inout float3 world) {if(index==0xffffffffu) {index=tap;}return float3(1.);}
    static uint sample_count() {return uint(frame.temporal.y)*max(frame.sampling.w,1u);}
    static float3 visibility_direction(float3 world,float3 normal,LightValues record,uint light,float3 direction,float distance,float3 position_error,uint index,uint count,bool fibre=false) {return hit_shadow_direction<P,ReflectionGeometry>(ray_scene,ray_alpha,light,world,normal,position_error,direction,distance,index,count,false);}
    static const bool subsurface=true;
    static float thickness(uint object,float3 world,float3 direction) {return subsurface_exit(ray_scene,object,world,direction);}
};
#ifdef VORK_REFLECTION_MEDIUM
import vork_medium;
import reflection_medium_scene;
import vork_medium_bounds;
import vork_medium_visibility_direct;
#ifdef VORK_REFLECTION_MEDIUM_GRID
import vork_medium_visibility_grid;
#ifdef VORK_METAL
struct ReflectionMediumPages {StructuredBuffer<float4> buffers[];};
[[vk::binding(2,3)]] ParameterBlock<ReflectionMediumPages> reflection_medium : register(VORK_REFLECTION_MEDIUM_REGISTER);
#define reflection_medium_page(page) reflection_medium.buffers[page]
#else
[[vk::binding(2,3)]] StructuredBuffer<float4> reflection_medium[];
#define reflection_medium_page(page) reflection_medium[NonUniformResourceIndex(page)]
#endif
export float4 medium_visibility_read(uint index) {
    return reflection_medium_page(index/VORK_REFLECTION_MEDIUM_CAPACITY)[index%VORK_REFLECTION_MEDIUM_CAPACITY];
}
export Frame medium_visibility_frame() {return frame;}
export uint medium_visibility_sample() {return 0u;}
export uint2 medium_size() {return vork_dimensions(receivers);}
#else
export float3 medium_cached_visibility(float3 position,uint light) {return float3(0.);}
#endif
static uint medium_sample_index;
export Frame medium_frame() {Frame f=frame;f.temporal.x=float(medium_sample_index);f.temporal.y*=float(max(frame.sampling.w,1u));return f;}
#ifdef VORK_MEDIUM_MESH_SOURCES
import vork_medium_mesh;
import vork_medium_shadow;
public float3 reflection_medium_mesh<P:IProgram>(float3 position,float3 ray,float g) {
    let f=medium_frame();
    return medium_mesh_incident<P,ReflectionGeometry>(ray_scene,ray_alpha,reflection_emitter_weights,f.medium.y,position,ray,g,medium_shadow_pixel(position),uint(f.temporal.x),uint(f.temporal.y));
}
#endif
export uint medium_word(uint index) {return material_words[index];}
export Light medium_light(uint index) {return lights[index];}
export float3 medium_environment_radiance(float3 direction) {return reflection_fallback(direction,0.,frame,env_specular,env_sampler);}
export uint medium_light_list(float3 position,out uint base) {base=0u;return uint(frame_light_count(frame));}
export uint medium_light_index(uint base,uint next) {return next;}
public float3 reflection_medium_visibility<P:IProgram>(float3 position,uint index) {
    return hit_shadow<P,ReflectionGeometry>(ray_scene,ray_alpha,index,position,float3(0.),float3(0.),uint(medium_frame().temporal.x),uint(medium_frame().temporal.y));
}
#endif
struct SurfaceHits<V:IVariant,P:IProgram> : IReflectionTrace {
    static float connection_probability() {return ray_alpha[reflection_range.z+1u]>0u ? .5 : 0.;}
    static float emitter_pdf(RayIntersection hit,float3x4 inverse,float3 from) {
        let base=reflection_range.z;let count=ray_alpha[base+1u];
        if(count==0u || ray_alpha[base+3u+hit.instance_index]==0xffffffffu) {return 0.;}
        let geometry=hit_emitter<ReflectionGeometry>(hit.instance_index,hit.primitive_index,hit.instance_custom_data,inverse,hit.barycentrics);
        return emitter_mesh_pdf<ReflectionGeometry>(base,hit.instance_index,geometry.area,from,geometry.point,geometry.face)/float(count);
    }
    // A separate source connection competes only with emitted radiance. Reflected
    // surface lighting has no source proposal and retains its ordinary weight.
    static float3 direction(ReflectionSite site,uint2 cell,uint index,float3 incident,float3 ray,float pdf,bool connection,EmitterTriangle selected) {
        let size=vork_dimensions(receivers);let footprint=connection ? float2(0.) : float2(1.)/float2(size);let receiver=site.receiver;
        let n=site.normal;
        let position=site.position;
        let geometric=site.geometric;
        let error=site.error;
        let origin=ray_origin(position,geometric,ray,error);
        RayDesc desc;
        desc.Origin=origin;desc.Direction=ray;
        desc.TMin=0.;desc.TMax=REG_FP32_MAX;
        // Find the nearest accepted distance with hardware pruning, then resolve
        // equal-distance coverage in a bounded interval. Committing the first
        // query may prune ties, so only the second query chooses their owner.
        DielectricHit closest={};closest.t=-1.;
        {
            RayQuery<RAY_FLAG_NONE> query;
            query.TraceRayInline(ray_scene,RAY_FLAG_NONE,REG_RAY_SPECULAR,desc);
            while (query.Proceed()) {
                if (query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE) {continue;}
                let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
                if (reflection_candidate_accepts<P,ReflectionGeometry>(ray_alpha,candidate,query.CandidateWorldToObject3x4(),ray,footprint,cell,index,ReflectionShadows<P>.sample_count())) {query.CommitNonOpaqueTriangleHit();}
            }
            if(query.CommittedStatus()==COMMITTED_TRIANGLE_HIT) {
                closest.t=query.CommittedRayT();closest.instance=query.CommittedInstanceIndex();closest.object=query.CommittedInstanceID();
                closest.primitive=query.CommittedPrimitiveIndex();closest.barycentrics=query.CommittedTriangleBarycentrics();closest.world_to_object=query.CommittedWorldToObject3x4();
            }
        }
        if(closest.t>=0.) {
            let record=material_records[objects[closest.object].material_index];
            var closest_coverage=record.tag.x!=0u || record.extra.y>0.;
            RayDesc ties=desc;
            // Adjacent representable endpoints keep the computed hit strictly
            // inside the interval; only exact equal-t candidates can replace it.
            ties.TMin=closest.t>0. ? asfloat(asuint(closest.t)-1u) : 0.;
            ties.TMax=min(asfloat(asuint(closest.t)+1u),REG_FP32_MAX);
            RayQuery<RAY_FLAG_FORCE_NON_OPAQUE> query;
            query.TraceRayInline(ray_scene,RAY_FLAG_NONE,REG_RAY_SPECULAR,ties);
            while(query.Proceed()) {
                if(query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE || query.CandidateTriangleRayT()!=closest.t) {continue;}
                let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
                let record=material_records[objects[candidate.instance_custom_data].material_index];
                let coverage=record.tag.x!=0u || record.extra.y>0.;
                if(closest_coverage && !coverage) {continue;}
                if(closest_coverage==coverage && (candidate.instance_custom_data>closest.object
                    || (candidate.instance_custom_data==closest.object && candidate.primitive_index>=closest.primitive))) {continue;}
                let inverse=query.CandidateWorldToObject3x4();
                if(!reflection_candidate_accepts<P,ReflectionGeometry>(ray_alpha,candidate,inverse,ray,footprint,cell,index,ReflectionShadows<P>.sample_count())) {continue;}
                closest.instance=candidate.instance_index;closest.object=candidate.instance_custom_data;
                closest.primitive=candidate.primitive_index;closest.barycentrics=candidate.barycentrics;closest.world_to_object=inverse;
                closest_coverage=coverage;
            }
        }
        closest=merge_field_hits<P,ReflectionGeometry>(closest,ray_fields(ray_alpha),origin,ray,desc.TMax,REG_RAY_SPECULAR);
        var transmission=float3(1.);var source=float3(0.);
#ifdef VORK_REFLECTION_MEDIUM
        let fraction=sample_2d_installed(sampling,frame.sampling.x,cell,index,sampling[frame.sampling.x],REG_SAMPLING_STREAM_VOLUME).x;
        let endpoint=closest.t<0. ? medium_infinity() : medium_receiver(closest.t);
        medium_sample_index=index;
        // Endpoint-emitter connections consume extinction only. Distributed fog
        // radiance belongs solely to the direction strategy and keeps its weight.
        let transport=connection
            ? volume_finite<VORK_MEDIUM_STEPS,false>(origin,ray,endpoint,fraction,0xffffffffu,false)
            : volume_finite<VORK_MEDIUM_STEPS,true,true>(origin,ray,endpoint,fraction,0xffffffffu,false);
        transmission=transport.transmission;
        // Only the BSDF strategy estimates distributed source radiance. The
        // mesh-source strategy competes only for endpoint emission, so medium
        // source must not receive its endpoint MIS weight or be counted twice.
        if(!connection) {source=transport.source;}
#endif
        let world_specular=site.world_specular;
        if (closest.t<0.) {
            // An escaped hardware ray sees the World rather than a local probe.
            return connection ? float3(0.) : (source+transmission*select(world_specular,directional_radiance(position,n,-incident,ray,0.,true,frame,env_specular,env_sampler),float3(0.)))*receiver.reflected*frame.radiance.w;
        }
        let hit=RayIntersection(closest.instance,closest.object,closest.primitive,closest.barycentrics);
        if(connection && (closest.field || hit.instance_index!=selected.instance || hit.primitive_index!=selected.primitive)) {return float3(0.);}
        if (ray_alpha[hit.instance_index*REG_RAY_SLOT_WORDS+2u]==REG_RAY_NO_ALPHA) {return source*receiver.reflected*frame.radiance.w;}
        let inverse=closest.world_to_object;
        let u=select(P.lobes,float(screen_ray_hash(cell,index,REG_SAMPLING_STREAM_RAY_GI_LOBE))*REG_RADICAL_INVERSE_SCALE,0.);
        var h=dielectric_surface<V,P,ReflectionGeometry>(closest,ray,footprint,Lobe(select(P.lobes,LOBE_PICK,0u),u));
        if(connection) {
            if(h.s.discarded || !material_covers_side<V>(h.v,h.entering)) {return float3(0.);}
            let emitted=emitter_mesh_radiance<V,P>(h);
            return transmission*emitted*receiver.reflected*frame.radiance.w;
        }
        // Non-source instances have no competing proposal. Builtin black
        // emission also needs no MIS geometry; retain conservative program support.
        let builtin=material_records[objects[h.v.object].material_index].tag.x==0u;
        let light_pdf=(!closest.field && (!builtin || any(h.s.closure.emission_color*h.s.closure.emission_luminance!=float3(0.)))) ? emitter_pdf(hit,inverse,position) : 0.;
        float3 emitted;
        let radiance=finalized_hit_radiance<V,ReflectionShadows<P>,P>(h,cell,index,REG_RAY_DIFFUSE|REG_RAY_SPECULAR,emitted);
        let mis=pdf/(pdf+light_pdf);
        return (source+transmission*(radiance-emitted+emitted*mis))*receiver.reflected*frame.radiance.w;
    }
    static float3 trace(ReflectionSite site,uint2 cell,uint index,float3 incident,float3 ray,float pdf) {
        EmitterTriangle selected={};
        return direction(site,cell,index,incident,ray,pdf,false,selected);
    }
    static float3 direct(ReflectionSite site,uint2 cell,uint index,float3 incident) {
        EmitterTriangle selected={};var value=float3(0.);
        let base=reflection_range.z;let count=ray_alpha[base+1u];
        if(count==0u) {return value;}
        // Discrete triangle choice and its residual are independent under the
        // uniform square. The residual and second coordinate sample its area.
        var u=sample_2d_installed(sampling,frame.sampling.x,cell,index,sampling[frame.sampling.x],REG_SAMPLING_STREAM_REFLECTION_EMITTER);
        let pick=min(uint(u.x*float(count)),count-1u);u.x=u.x*float(count)-float(pick);
        selected=emitter_mesh_triangle<ReflectionGeometry>(base,pick);
        let radius=sqrt(u.x);selected.barycentrics=float2(radius*(1.-u.y),radius*u.y);
        let inverse=(float3x4)object_normal(selected.object);
        let geometry=hit_emitter<ReflectionGeometry>(selected.instance,selected.primitive,selected.object,inverse,selected.barycentrics);
        let axis=normalize(geometry.point-site.position);
        if(dot(axis,site.geometric)<=0.) {return value;}
        let light_pdf=emitter_mesh_pdf<ReflectionGeometry>(base,selected.instance,geometry.area,site.position,geometry.point,geometry.face)/float(count);
        if(light_pdf<=0.) {return value;}
        float bsdf_pdf;let response=reflection_bsdf_evaluate(site.bsdf,-incident,axis,bsdf_pdf);
        let mixture_pdf=bsdf_pdf;
        site.receiver.reflected=response/(light_pdf+mixture_pdf);
        if(any(site.receiver.reflected!=float3(0.))) {value+=direction(site,cell,index,incident,axis,mixture_pdf,true,selected);}
        return value;
    }
};
// Scenes with programs compile their own entry with their Programs (SlangForward::program_kernel).
public void reflections_pixel<V:IVariant,P:IProgram>(uint3 gid) {
    let size=vork_dimensions(receivers);
    if(gid.x>=REG_REFLECTION_DISPATCH_PIXELS) {return;}
    let pixel=(reflection_range.w&~REG_REFLECTION_MERGED_SLOTS)+gid.x;
    if(pixel>=size.x*size.y) {return;}
    gid.xy=uint2(pixel%size.x,pixel/size.x);
    reflection_integrate<SurfaceHits<V,P>>(gid,frame,vork_dimensions(receivers),reflection_slots,reflection_records,reflection_guides,sampling,reflection_range.xy,reflection_accumulation,output,(reflection_range.w&REG_REFLECTION_MERGED_SLOTS)!=0u);
}
[shader("compute")]
[numthreads(REG_COMPUTE_TILE*REG_COMPUTE_TILE,1,1)]
void compute(uint3 gid : SV_DispatchThreadID) {reflections_pixel<Unspecialized,NoPrograms>(gid);}
