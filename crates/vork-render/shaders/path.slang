// Unidirectional ray-query transport, using the installed geometry, material
// table and light buffer. Raster passes still produce the control channels.
import vork_constants;
import frame;
import catcher;
#ifdef VORK_SPATIAL_ENVIRONMENT
import spatial_environment;
#endif
import light_tree;
import closure;
import material_layout;
import variant;
import shadows;
import material;
import openpbr_inputs;
import bsdf;
import emitter;
import emitter_mesh;
import emitter_implicit;
import openpbr_core;
import vork_ray_alpha;
import vork_shadow_coverage;
import surface;
import hair_lighting;
import hit;
import ray_coverage;
import forward;
import subsurface_transport;
import path_boundary;
import sample_clamp;
import dielectric_scene;
import implicit_scene;
import path_subsurface;
import environment_proposal;
import path_sampler;
#include "path_work_arguments.slang"

// boundaries.xy are descriptor offsets; .z is the absolute first row; .w is the
// ray-table offset of the per-solid object-space occupancy bounds.
struct PathSettings { uint4 bounces; uint4 limits; float4 clamp; uint4 sampling; float4 adaptive; uint4 boundaries; uint4 environment; uint4 implicit; uint4 matte; uint4 propagation; };
// settled: the pixel's noise test on even/odd samples, read by its neighbours;
// stopped: it has stopped sampling for this exposure.
struct PathMoment { float4 mean; float4 variance; uint2 settled; uint stopped; uint padding; };
[[vk::binding(10,0)]] RaytracingAccelerationStructure path_scene : register(t5);
[[vk::binding(11,0)]] StructuredBuffer<uint> path_words : register(t6);
[[vk::binding(12,0)]] ConstantBuffer<PathSettings> path_settings : register(b7);
[[vk::binding(13,0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> path_output : register(u4);
// Raster geometry buffers ray hits read in place (rt::hit_record, rt::hits_layout).
#ifdef VORK_METAL
[[vk::binding(0,3)]] ParameterBlock<RayHitBuffersTable> path_hit_buffers : register(VORK_HIT_REGISTER);
#else
[[vk::binding(0,3)]] StructuredBuffer<uint> path_hit_buffers[REG_RAY_HIT_BUFFERS];
#endif
uint path_hit_buffers_load(uint buffer,uint index) {
#ifdef VORK_METAL
    return path_hit_buffers.buffers[buffer][index];
#else
    return path_hit_buffers[NonUniformResourceIndex(buffer)][index];
#endif
}
struct PathGeometry : IHitGeometry {
    static uint word(uint index) {return path_words[index];}
    static uint load(uint buffer,uint index) {return path_hit_buffers_load(buffer,index);}
};
#ifdef VORK_PROGRAM_TRACE
import geometric_trace;
export TraceHit native_geometric_trace(float3 origin,float3 direction,float minimum,float maximum,uint mask,uint object) {
    return geometric_trace<PathGeometry>(path_scene,origin,direction,minimum,maximum,mask,object);
}
#endif

uint2 path_image_size() {uint width,height;path_output.GetDimensions(width,height);return uint2(width,height);}
[[vk::binding(14,0)]] StructuredBuffer<LightTreeNode> path_light_nodes : register(t8);
[[vk::binding(15,0)]] Texture2DArray<uint4> path_environment_source : register(t5);
[[vk::binding(16,0)]] SamplerState path_environment_sampler : register(s1);
[[vk::binding(17,0)]] StructuredBuffer<float2> path_environment_distribution : register(t9);
[[vk::binding(18,0)]] StructuredBuffer<float> path_emitter_weights : register(t10);
[[vk::binding(19,0)]] RWStructuredBuffer<float4> path_aovs : register(u11);
[[vk::binding(20,0)]] RWStructuredBuffer<float4> path_guides : register(u12);
[[vk::binding(21,0)]] [[vk::image_format("rg32f")]] RWTexture2D<float2> field_depth : register(u6);
[[vk::binding(22,0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> field_normals : register(u7);
[[vk::binding(23,0)]] [[vk::image_format("r32ui")]] RWTexture2D<uint> field_ids : register(u8);
#ifndef VORK_CATCHER_REGISTER
#define VORK_CATCHER_REGISTER u13
#define VORK_CATCHER_PIXELS_REGISTER u14
#endif
[[vk::binding(24,0)]] RWStructuredBuffer<float4> path_catcher : register(VORK_CATCHER_REGISTER);
[[vk::binding(25,0)]] RWStructuredBuffer<uint> path_pixels : register(VORK_CATCHER_PIXELS_REGISTER);
// Last, so fixed sampling's layout stays contiguous without it.
#ifdef VORK_PATH_ADAPTIVE_NATIVE
[[vk::binding(26,0)]] RWStructuredBuffer<PathMoment> path_moments : register(u15);
#else
[[vk::binding(26,0)]] RWStructuredBuffer<PathMoment> path_moments;
#endif

#ifdef VORK_LIGHT_PATHS
import camera_dispatch;
import vork_spectrum;
static PathSensorContext path_sensor_context;
static float path_wavelength_scale;
float3 path_arrival(float optical_length) {
    var ctx=path_sensor_context;ctx.path_length=optical_length;
    return ((frame.camera_program.z&1u)!=0u ? spectral_response(ctx.wavelength) : float3(1.))*sensor_dispatch(frame.camera_program.x,frame.camera_program.y,ctx);
}
float3 path_spectrum(float3 rgb,bool emission=false) {return (frame.camera_program.z&1u)!=0u ? spectral_value(rgb,path_sensor_context.wavelength*path_wavelength_scale,emission) : rgb;}
#else
float3 path_spectrum(float3 rgb,bool emission=false) {return rgb;}
float3 path_arrival(float optical_length) {return float3(1.);}
#endif
float3 connection_arrival(float prefix,float distance,float ior) {
    return path_arrival(distance>=REG_FP32_MAX ? REG_FP32_MAX : prefix+distance*ior);
}

// Bound Slang's optimization graph; SPIR-V handoff restores driver inlining freedom.
#if defined(VORK_PATH_SLANG_OUTLINE)
#define VORK_PATH_HELPER [noinline]
#else
#define VORK_PATH_HELPER
#endif

// The installed geometry capability removes the entire latent field graph,
// including auxiliary camera queries and emitter/containment consumers.
uint4 path_fields<V:IVariant>() {
    if (!V.has_implicit_geometry) {return uint4(0u);}
    return path_settings.implicit;
}

VORK_PATH_HELPER
DielectricHit trace<V:IVariant,P:IProgram>(float3 origin, float3 direction, float distance, uint mask VORK_WORK_PARAMETER) {
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=distance;
#ifdef VORK_PROPAGATION
    // Adjacent segments own their shared endpoint. Native triangle queries
    // may exclude TMax and reject a zero-distance hit in the following segment.
    ray.TMax=distance<REG_FP32_MAX ? asfloat(asuint(distance)+1u) : distance;
#endif
    // BLAS opacity proves built-in surface acceptance. Programmed coverage
    // can differ from shadow opacity,
    // so retain its candidates until installation proves surface opacity too.
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(path_scene,select(P.coverage || catcher_enabled,RAY_FLAG_FORCE_NON_OPAQUE,RAY_FLAG_NONE),mask,ray);
    while (query.Proceed()) {
#ifdef VORK_MEDIUM_WORK_LIMIT
        // Stage 1 admits builtin surface callbacks only. One candidate and its
        // bounded coverage/closure evaluation consume a separate credit.
        if(!medium_work_reserve(medium_work,1u,7u,0u)) {break;}
#endif
        if (query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE) {continue;}
        if (!catcher_keeps(query.CandidateInstanceID())) {continue;}
        let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        uint width,height;path_output.GetDimensions(width,height);
#ifdef VORK_PATH_MEDIA
        if((material_records[material_record(query.CandidateInstanceID())].tag.z&REG_FLAG_MEDIUM_BOUNDARY)!=0u) {continue;}
#endif
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_alpha_reserve(medium_work,1u,candidate.instance_custom_data)) {break;}
#endif
        let accepted=candidate_accepts<P,PathGeometry>(path_words,candidate,query.CandidateWorldToObject3x4(),direction,float2(2./float(width),2./float(height)));
        if (accepted) {query.CommitNonOpaqueTriangleHit();}
    }
    DielectricHit h={};h.t=-1.;
    if (query.CommittedStatus()==COMMITTED_TRIANGLE_HIT) {
        h.t=query.CommittedRayT();h.instance=query.CommittedInstanceIndex();h.primitive=query.CommittedPrimitiveIndex();
        h.object=query.CommittedInstanceID();h.barycentrics=query.CommittedTriangleBarycentrics();h.world_to_object=query.CommittedWorldToObject3x4();
    }
    h=merge_field_hits<P,PathGeometry>(h,path_fields<V>().xy,origin,direction,distance,mask);
    return h;
}
#define VORK_PATH_CONTEXT
#include "path_propagation.slang"
#ifdef VORK_PATH_MEDIA
import path_medium;
import vork_medium_point;
import vork_medium_domain;
export Frame medium_frame() {return frame;}
export uint medium_word(uint index) {return material_words[index];}
import path_medium_scene;
// The interval is a per-path occupancy snapshot. Additive participating media
// stay independent of the exclusive dielectric owner in PathOccupancy.
struct PathMediumInterval {
    MediumSample components[VORK_PATH_MEDIUM_BOUNDS+1];
    float end;
    float3 extinction;
    float3 emission;
    float scattering_power;
    uint component_groups[VORK_PATH_MEDIUM_BOUNDS+1];
    uint receiver_groups;
#if VORK_PATH_HETEROGENEOUS
    uint programs[VORK_PATH_MEDIUM_BOUNDS+1];
    uint input_bases[VORK_PATH_MEDIUM_BOUNDS+1];
    uint component_objects[VORK_PATH_MEDIUM_BOUNDS+1];
    float majorant;
    bool varying;
    bool exponential;
    bool valid;
#endif
    float3 linked_phase(float cosine,uint contribution,uint groups,bool delta=false) {
        var value=float3(0.);
        for(var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
            let m=components[i];
            if(light_receives(contribution,groups,component_groups[i])) {
                value+=m.scattering*(delta ? (m.anisotropy==cosine ? 1. : 0.) : medium_phase(m.anisotropy,cosine));
            }
        }
        return value;
    }
    float3 phase(float cosine) {
        var value=float3(0.);
        for (var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
            let m=components[i];value+=m.scattering*medium_phase(m.anisotropy,cosine);
        }
        return value;
    }
    float pdf(float cosine) {return scattering_power>0. ? dot(phase(cosine),REG_LUMINANCE)/scattering_power : 0.;}
};
bool valid_medium_point(MediumSample m) {
    return all(isfinite(m.absorption)) && all(isfinite(m.scattering)) && all(isfinite(m.emission)) && isfinite(m.anisotropy) && all(m.absorption>=float3(0.)) && all(m.scattering>=float3(0.)) && all(m.emission>=float3(0.)) && abs(m.anisotropy)<1.;
}
void medium_component(inout PathMediumInterval interval,uint slot,uint id,uint object,float3 position,MediumTransportContext context) {
    var m=path_medium_coefficients(id,position,context);
    if(!valid_medium_point(m)) {
        path_boundary_error(REG_PATH_MEDIUM_INVALID,id);m=MediumSample();
    }
    m.absorption=path_spectrum(m.absorption);m.scattering=path_spectrum(m.scattering);m.emission=path_spectrum(m.emission,true);interval.components[slot]=m;
#if VORK_PATH_HETEROGENEOUS
    interval.programs[slot]=id;interval.input_bases[slot]=context.input_base;interval.component_objects[slot]=object;
    if(slot==0u) {
        float3 a,b,g;interval.exponential=path_world_exponential(a,b,g);
        if(interval.exponential) {
            m.absorption=medium_exponential_coefficient(path_spectrum(a),dot(g,position));
            m.scattering=medium_exponential_coefficient(path_spectrum(b),dot(g,position));
            interval.components[slot]=m;interval.varying=true;return;
        }
    }
    let sigma=m.absorption+m.scattering;let constant=path_medium_homogeneous(id);
    let bound=constant ? medium_sample_majorant(m.absorption,m.scattering) : asfloat(material_words[context.input_base-1u]);
    interval.varying|=!constant;
    if(!isfinite(bound) || bound<0. || !all(isfinite(sigma)) || !all(isfinite(m.emission)) || any(m.absorption<float3(0.)) || any(m.scattering<float3(0.))) {path_boundary_error(REG_PATH_MEDIUM_MAJORANT,0u);interval.valid=false;}
    else if(any(sigma>float3(bound))) {path_boundary_error(REG_PATH_MEDIUM_MAJORANT,asuint(max(sigma.x,max(sigma.y,sigma.z))));interval.valid=false;}
#endif
}
void medium_sum(inout PathMediumInterval interval,bool derive_bound=false) {
    MediumAggregate sum={};
#if VORK_PATH_HETEROGENEOUS
    if(derive_bound) {interval.majorant=0.;}
#endif
    for(var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
        medium_add(sum,interval.components[i]);
#if VORK_PATH_HETEROGENEOUS
        if(derive_bound && interval.programs[i]!=0xffffffffu && !(i==0u && interval.exponential)) {
            let m=interval.components[i];
            let bound=path_medium_homogeneous(interval.programs[i])
                ? medium_sample_majorant(m.absorption,m.scattering)
                : asfloat(material_words[interval.input_bases[i]-1u]);
            interval.majorant=medium_majorant_add(interval.majorant,bound);
        }
#endif
    }
    interval.extinction=sum.extinction;interval.emission=sum.emission;interval.scattering_power=sum.scattering_power;
}
#if VORK_PATH_HETEROGENEOUS
PathMediumInterval medium_at(PathMediumInterval interval,float3 position VORK_WORK_PARAMETER) {
#ifdef VORK_MEDIUM_WORK_LIMIT
    if(!medium_work_reserve(medium_work,1u+(VORK_PATH_MEDIUM_BOUNDS+1u)*VORK_MEDIUM_POINT_WORK,5u,0u)) {interval.valid=false;return interval;}
#endif
    for(var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
        let id=interval.programs[i];if(id==0xffffffffu || path_medium_homogeneous(id)) {continue;}
        let object=interval.component_objects[i];MediumTransportContext context={};context.input_base=interval.input_bases[i];
        context.inverse=object==0xffffffffu ? float4x4(float4(1.,0.,0.,0.),float4(0.,1.,0.,0.),float4(0.,0.,1.,0.),float4(0.,0.,0.,1.)) : transpose(object_normal(object));
        var m=path_medium_coefficients(id,position,context);
        if(!valid_medium_point(m)) {path_boundary_error(REG_PATH_MEDIUM_INVALID,id);m=MediumSample();}
        m.absorption=path_spectrum(m.absorption);m.scattering=path_spectrum(m.scattering);m.emission=path_spectrum(m.emission,true);interval.components[i]=m;
        if(i==0u && interval.exponential) {
            float3 a,b,g;path_world_exponential(a,b,g);
            m.absorption=medium_exponential_coefficient(path_spectrum(a),dot(g,position));
            m.scattering=medium_exponential_coefficient(path_spectrum(b),dot(g,position));
            interval.components[i]=m;continue;
        }
        let bound=asfloat(material_words[context.input_base-1u]);let sigma=m.absorption+m.scattering;
        if(!all(isfinite(sigma)) || !all(isfinite(m.emission)) || any(m.absorption<float3(0.)) || any(m.scattering<float3(0.))) {path_boundary_error(REG_PATH_MEDIUM_MAJORANT,0u);interval.valid=false;}
        else if(any(sigma>float3(bound))) {path_boundary_error(REG_PATH_MEDIUM_MAJORANT,asuint(max(sigma.x,max(sigma.y,sigma.z))));interval.valid=false;}
    }
    medium_sum(interval);return interval;
}
float medium_candidate(float cursor,float majorant,inout uint rng VORK_WORK_PARAMETER) {
#ifdef VORK_MEDIUM_WORK_LIMIT
    if(!medium_work_reserve(medium_work,1u,6u,0u)) {return REG_FP32_MAX;}
#endif
    let step=-log(1.-path_random(rng))/majorant;
    // IEEE progress when a free-flight step is below the accumulated distance's
    // representable spacing; no density threshold or collision-count cutoff.
    return max(cursor+step,asfloat(asuint(cursor)+1u));
}
#endif
#if VORK_PATH_HETEROGENEOUS
float medium_interval_candidate(PathMediumInterval medium,float3 origin,float3 direction,float cursor,inout uint rng VORK_WORK_PARAMETER) {
    if(!medium.exponential) {return medium_candidate(cursor,medium.majorant,rng VORK_WORK_ARGUMENT);}
#ifdef VORK_MEDIUM_WORK_LIMIT
    // Mixed constant/exponential inversion halves a positive FP32 interval.
    // 512 subdivisions conservatively cover its exponent and significand range;
    // the world-only inverse is closed form. Reserve before either executes.
    if(!medium_work_reserve(medium_work,medium.majorant>0. ? 513u : 1u,6u,0u)) {return REG_FP32_MAX;}
#endif
    float3 a,b,g;path_world_exponential(a,b,g);
    let amplitude=medium_sample_majorant(path_spectrum(a),path_spectrum(b));
    let optical=-log(1.-path_random(rng));
    let step=medium_exponential_candidate(medium.majorant,amplitude,dot(g,origin+direction*cursor),dot(g,direction),optical,medium.end-cursor);
    if(step>=medium.end-cursor) {return medium.end;}
    return max(cursor+step,asfloat(asuint(cursor)+1u));
}
float medium_interval_rate(PathMediumInterval medium,float3 position) {
    if(!medium.exponential) {return medium.majorant;}
    float3 a,b,g;path_world_exponential(a,b,g);
    return medium.majorant+medium_exponential_coefficient(float3(medium_sample_majorant(path_spectrum(a),path_spectrum(b))),dot(g,position)).x;
}
float3 medium_finite_extinction(PathMediumInterval medium) {
    var sigma=float3(0.);
    for(uint i=medium.exponential ? 1u : 0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {sigma+=medium.components[i].absorption+medium.components[i].scattering;}
    return sigma;
}
#endif
PathMediumInterval medium_interval(float3 origin,float3 direction,float after,float limit,bool camera_ray VORK_WORK_PARAMETER) {
    PathMediumInterval result={};result.end=limit;
#ifdef VORK_MEDIUM_WORK_LIMIT
    // Two domain scans, each bounded to 32 faces per admitted convex carrier,
    // plus one bounded coefficient callback per domain. Reserve interval and
    // continuation work together, before either executes. Shadow intervals
    // conservatively pay the continuation unit too.
    let domains=volume_domain_count();
    if(!medium_work_reserve(medium_work,2u+domains*(64u+VORK_MEDIUM_POINT_WORK)+((frame.medium.w&1u)!=0u ? VORK_MEDIUM_POINT_WORK : 0u),1u,0u)) {return result;}
#endif
#if VORK_PATH_HETEROGENEOUS
    result.valid=true;
    for(var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {result.programs[i]=0xffffffffu;result.component_objects[i]=0xffffffffu;}
#endif
    result.end=volume_interval_end(origin,direction,after,limit,0xffffffffu,camera_ray);
    for(uint i=0u;i<=VORK_PATH_MEDIUM_BOUNDS;i++) {
        let instance=volume_instance(i,origin,direction,after,limit,0xffffffffu,camera_ray);
        if(instance.program==0xffffffffu || (instance.object!=0xffffffffu && !catcher_keeps(instance.object))) {continue;}
        let slot=i==VORK_PATH_MEDIUM_BOUNDS ? 0u : i+1u;
        if(instance.object!=0xffffffffu) {
            result.component_groups[slot]=objects[instance.object].metadata.x;
            result.receiver_groups|=result.component_groups[slot];
        }
        medium_component(result,slot,instance.program,instance.object,origin+direction*after,instance.context);
    }
    medium_sum(result,true);
#if VORK_PATH_HETEROGENEOUS
    if(!isfinite(result.majorant)) {path_boundary_error(REG_PATH_MEDIUM_MAJORANT,0u);result.valid=false;}
#endif
    return result;
}
float3 straight_participating_transmittance(float3 origin,float3 direction,float distance,inout uint rng VORK_WORK_PARAMETER) {
    var attenuation=float3(1.);var cursor=0.;
    while(cursor<distance) {
        let segment=medium_interval(origin,direction,cursor,distance,false VORK_WORK_ARGUMENT);
#if VORK_PATH_HETEROGENEOUS
        if(!segment.valid) {return float3(0.);}
        if(segment.exponential) {
            float3 a,b,g;path_world_exponential(a,b,g);
            let tau=medium_exponential_tau(path_spectrum(a)+path_spectrum(b),dot(g,origin+direction*cursor),dot(g,direction),segment.end-cursor,segment.end==REG_FP32_MAX);
            attenuation*=exp(-tau);
            if(!any(attenuation>float3(0.))) {return float3(0.);}
        }
        if(segment.varying) {
            if(segment.majorant>0.) {
                var candidate=medium_candidate(cursor,segment.majorant,rng VORK_WORK_ARGUMENT);
                while(candidate<segment.end) {
                    let point=medium_at(segment,origin+direction*candidate VORK_WORK_ARGUMENT);
                    if(!point.valid) {return float3(0.);}
                    attenuation*=float3(1.)-medium_finite_extinction(point)/segment.majorant;
                    if(!any(attenuation>float3(0.))) {return float3(0.);}
                    candidate=medium_candidate(candidate,segment.majorant,rng VORK_WORK_ARGUMENT);
                }
            }else if(!segment.exponential){
                // Zero is a valid estimated bound for pure varying emission.
                // Check a sampled point even when no Poisson candidates exist.
                let point=medium_at(segment,origin+direction*lerp(cursor,segment.end,path_random(rng)) VORK_WORK_ARGUMENT);if(!point.valid) {return float3(0.);}
            }
        }else
#endif
        {attenuation*=segment.end==REG_FP32_MAX ? medium_infinite_transmittance(segment.extinction) : exp(-segment.extinction*(segment.end-cursor));}
        cursor=segment.end;
    }
    return attenuation;
}
struct PathPhaseDirection {float3 direction;float3 weight;float pdf;float measure;float cosine;};
// u: x the phase cosine, y the azimuth, z the component pick (path_scatter's convention).
PathPhaseDirection phase_direction(PathMediumInterval medium,float3 incident,float3 u) {
    PathPhaseDirection result={};if(medium.scattering_power<=0.) {return result;}
    var select=u.z*medium.scattering_power;var selected=0u;
    for (var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
        let power=dot(medium.components[i].scattering,REG_LUMINANCE);
        if(power>0.) {selected=i;}if (select<power) {break;}select-=power;
    }
    let phase=medium_phase_sample(medium.components[selected].anisotropy,u.x);result.cosine=phase.cosine;
    if(phase.delta) {
        result.direction=incident*phase.cosine;var mass=0.;var value=float3(0.);
        for (var i=0u;i<VORK_PATH_MEDIUM_BOUNDS+1u;i++) {
            let m=medium.components[i];if(m.anisotropy==phase.cosine) {mass+=dot(m.scattering,REG_LUMINANCE);value+=m.scattering;}
        }
        result.measure=mass/medium.scattering_power;result.weight=value/result.measure;return result;
    }
    let tangent=normalize(cross(abs(incident.z)<abs(incident.x) ? float3(0.,0.,1.) : float3(1.,0.,0.),incident));
    let bitangent=cross(incident,tangent);let radius=sqrt(max(1.-phase.cosine*phase.cosine,0.));let phi=REG_TAU*u.y;
    result.direction=incident*phase.cosine+radius*(tangent*cos(phi)+bitangent*sin(phi));
    result.pdf=medium.pdf(phase.cosine);result.measure=result.pdf;result.weight=medium.phase(phase.cosine)/result.pdf;return result;
}
// Return a real scattering vertex, or finish the complete segment. True null
// boundaries never spend a surface/volume bounce or an alpha crossing budget.
bool advance_medium(float3 origin,float3 direction,float limit,PathBoundary optical,bool camera_ray,float optical_length,inout float3 throughput,inout float3 radiance,inout uint rng,out float distance,out PathMediumInterval vertex VORK_WORK_PARAMETER) {
    var cursor=0.;vertex={};distance=limit;
    while(cursor<limit) {
        var medium=medium_interval(origin,direction,cursor,limit,camera_ray VORK_WORK_ARGUMENT);
        let length=medium.end-cursor;
#if VORK_PATH_HETEROGENEOUS
        if(!medium.valid) {throughput=float3(0.);return false;}
#ifdef VORK_MEDIUM_ABSORPTION_ONLY
        if(medium.exponential) {
            float3 a,b,g;path_world_exponential(a,b,g);
            let tau=medium_exponential_tau(path_spectrum(a),dot(g,origin+direction*cursor),dot(g,direction),length,medium.end==REG_FP32_MAX);
            throughput*=exp(-tau-medium_finite_extinction(medium)*length)*boundary_attenuation(optical,length);
            cursor=medium.end;continue;
        }
#endif
        if(medium.varying) {
            if(medium.exponential && medium.end==REG_FP32_MAX) {
                float3 a,b,g;path_world_exponential(a,b,g);
                let unattenuated=dot(g,direction)<0. ? medium.components[0].emission : select((a+b)==float3(0.),medium.components[0].emission,float3(0.));
                if(any(unattenuated>float3(0.))) {path_boundary_error(REG_PATH_MEDIUM_DIVERGENT,0xffffffffu);throughput=float3(0.);return false;}
            }
            if(medium.majorant==0. && !medium.exponential) {
                let sample=length*path_random(rng);let point=medium_at(medium,origin+direction*(cursor+sample) VORK_WORK_ARGUMENT);
                if(!point.valid) {throughput=float3(0.);return false;}
                radiance+=throughput*point.emission*boundary_attenuation(optical,sample)*length*path_arrival(optical_length+(cursor+sample)*optical.ior);
                throughput*=boundary_attenuation(optical,length);cursor=medium.end;continue;
            }
            while(cursor<medium.end) {
                let candidate=medium_interval_candidate(medium,origin,direction,cursor,rng VORK_WORK_ARGUMENT);
                let stop=min(candidate,medium.end);throughput*=boundary_attenuation(optical,stop-cursor);cursor=stop;
                if(candidate>=medium.end) {break;}
                let point=medium_at(medium,origin+direction*candidate VORK_WORK_ARGUMENT);
                if(!point.valid) {throughput=float3(0.);return false;}
                let rate=medium_interval_rate(medium,origin+direction*candidate);
                radiance+=throughput*point.emission/rate*path_arrival(optical_length+candidate*optical.ior);
                let peak=max(point.extinction.x,max(point.extinction.y,point.extinction.z));
                let extinction=peak>0. ? peak*((point.extinction.x/peak+point.extinction.y/peak+point.extinction.z/peak)/3.) : 0.;
                let real=extinction/rate;
                if(path_random(rng)<real) {throughput/=extinction;distance=candidate;vertex=point;return true;}
                throughput*=(float3(1.)-point.extinction/rate)/(1.-real);
                if(!any(throughput>float3(0.))) {return false;}
            }
            continue;
        }
#endif
        let infinite=medium.end==REG_FP32_MAX;
        let attenuation=infinite ? medium_infinite_transmittance(medium.extinction) : exp(-medium.extinction*length);
        var emission_extinction=medium.extinction;
        if(optical.depth>0.) {emission_extinction+=-log(select(optical.color>float3(0.),optical.color,float3(1.)))/optical.depth;}
        var unattenuated_emission=medium.emission;
        if(optical.depth>0.) {unattenuated_emission=select(optical.color>float3(0.),unattenuated_emission,float3(0.));}
        if(medium.end==REG_FP32_MAX && any(select(emission_extinction==float3(0.),unattenuated_emission,float3(0.))>float3(0.))) {
            path_boundary_error(REG_PATH_MEDIUM_DIVERGENT,0xffffffffu);throughput=float3(0.);return false;
        }
        var emission=infinite ? medium_equilibrium(medium.emission,MediumExtinction(float3(1.),emission_extinction))
            : medium_integrated_coefficient(medium.emission,emission_extinction,length,exp(-emission_extinction*length));
        if(optical.depth>0.) {emission=select(optical.color>float3(0.),emission,float3(0.));}
#ifdef VORK_LIGHT_PATHS
        // Only detector programs need a sampled arrival distance. Ordinary
        // camera programs retain the exact homogeneous emission integral.
        if((frame.camera_program.z&4u)!=0u && length<REG_FP32_MAX && any(medium.emission!=float3(0.))) {
            let t=length*path_random(rng);
            radiance+=throughput*medium.emission*exp(-emission_extinction*t)*length*path_arrival(optical_length+(cursor+t)*optical.ior);
        } else {
            radiance+=throughput*emission*path_arrival(REG_FP32_MAX);
        }
#else
        radiance+=throughput*emission;
#endif

        if(medium.scattering_power>0.) {
            let ux=path_random(rng);let uy=path_random(rng);
            let flight=medium_free_flight(medium.extinction,length,float2(ux,uy),infinite);
            if(flight.event && !isfinite(flight.distance)) {path_boundary_error(REG_PATH_MEDIUM_DISTANCE,0u);throughput=float3(0.);return false;}
            throughput*=flight.weight*boundary_attenuation(optical,flight.distance);
            if(flight.event) {distance=cursor+flight.distance;vertex=medium;return true;}
        }else{throughput*=attenuation*boundary_attenuation(optical,length);}
        cursor=medium.end;
    }
    return false;
}
#else
float3 straight_participating_transmittance(float3 origin,float3 direction,float distance,inout uint rng VORK_WORK_PARAMETER) {return float3(1.);}
#endif

float3 participating_transmittance(float3 origin,float3 direction,float distance,inout uint rng VORK_WORK_PARAMETER) {
#ifndef VORK_PROPAGATION
    return straight_participating_transmittance(origin,direction,distance,rng VORK_WORK_ARGUMENT);
#else
    var escaped=false;var previous_distance=0.;var result=float3(1.);var remaining=distance;
    for(uint work=0u;work<REG_PROPAGATION_WORK_LIMIT;work++) {
        float limit;float3 transfer;
        if(!propagation_segment(origin,direction,escaped,previous_distance,limit,transfer)) return float3(0.);
        limit=min(limit,remaining);
        result*=straight_participating_transmittance(origin,direction,limit,rng VORK_WORK_ARGUMENT);
        if(limit==REG_FP32_MAX || limit>=remaining) return result;
        origin+=direction*limit;remaining-=limit;
    }
    path_boundary_error(REG_PATH_PROPAGATION_ERROR,3u);return float3(0.);
#endif
}

// RGB straight-segment transmittance through the common candidate evaluator.
// Evaluated alpha zero transmits the connection.
// Path retains its incident IOR, mixed-closure sampling and transparent budget.
VORK_PATH_HELPER
float3 straight_shadow_transmittance<V:IVariant,P:IProgram>(float3 origin, float3 direction, float distance, uint groups, bool all_casters, uint transparent, float incident_ior VORK_WORK_PARAMETER) {
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=distance;
#ifdef VORK_PROPAGATION
    // Adjacent segments own their shared endpoint. Native triangle queries
    // may exclude TMax and reject a zero-distance hit in the following segment.
    ray.TMax=distance<REG_FP32_MAX ? asfloat(asuint(distance)+1u) : distance;
#endif
    // Thin sheets can carry a discrete null component even in opaque BLASes.
    // Visit their material before deciding whether the connection is blocked.
    var transmittance=float3(1.);
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;
    query.TraceRayInline(path_scene,select(all_casters && !V.has_transmission && !catcher_enabled,RAY_FLAG_NONE,RAY_FLAG_FORCE_NON_OPAQUE),REG_RAY_SHADOW,ray);
    while (query.Proceed()) {
#ifdef VORK_MEDIUM_WORK_LIMIT
        // Stage 1 admits builtin surface callbacks only. One candidate and its
        // bounded coverage/closure evaluation consume a separate credit.
        if(!medium_work_reserve(medium_work,1u,7u,0u)) {break;}
#endif
        if (query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE) {continue;}
        let object=query.CandidateInstanceID();
        if (!catcher_keeps(object)) {continue;}
        if (!all_casters && !links_intersect(objects[object].metadata.x,groups)) {continue;}
        let hit=RayIntersection(query.CandidateInstanceIndex(),object,query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        uint width,height;path_output.GetDimensions(width,height);
        if (!connection_owns_hit<PathGeometry>(hit,query.CandidateWorldToObject3x4(),direction)) {continue;}
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_alpha_reserve(medium_work,2u,object)) {return float3(0.);}
#endif
        let crossing=connection_candidate_transmittance<P,PathGeometry>(path_words,hit,query.CandidateWorldToObject3x4(),direction,float2(2./float(width),2./float(height)),origin,uint(frame.temporal.x),incident_ior,V.has_transmission);
        if (!any(crossing.transmittance>float3(0.))) {return float3(0.);}
        if (crossing.transparent_crossings!=0u && transparent++>=path_settings.limits.z) {return float3(0.);}
        transmittance*=path_spectrum(crossing.transmittance);
    }
    if(query.CommittedStatus()!=COMMITTED_NOTHING) return float3(0.);
    for(uint i=0u;i<path_fields<V>().y;i++) {
        let at=path_fields<V>().x+i*3u;let object=path_words[at+1u];
        if(!catcher_keeps(object) || collapsed_endpoint(objects[object].model) || (objects[object].metadata.y&REG_RAY_SHADOW)==0u || (!all_casters && (objects[object].metadata.x&groups)==0u)) continue;
        var minimum=0.;
        for(;;) {
            let root=field_intersection<PathGeometry>(path_words[at+2u],object,origin,direction,minimum,distance);
            if(!root.found) break;
            let h=field_surface<V,P>(object,root,direction,float2(0.),Lobe(0u,0.));
            if(!h.s.discarded) {
                let opacity=saturate(h.s.closure.geometry_opacity);
                let sheet=transport_null_transmittance(h.s.closure,-direction,incident_ior,h.s.geometric,h.face)*h.s.weight;
                transmittance*=path_spectrum((1.-opacity)+opacity*sheet);
                if(!any(transmittance>float3(0.))) return float3(0.);
                if(transparent++>=path_settings.limits.z) return float3(0.);
            }
            minimum=root.t+REG_FP32_GAMMA7*max(1.,abs(root.t));
            if(minimum>=distance) break;
        }
    }
    return transmittance;
}
float3 path_shadow_transmittance<V:IVariant,P:IProgram>(float3 origin,float3 direction,float distance,uint groups,bool all_casters,uint transparent,float incident_ior,bool occlusion VORK_WORK_PARAMETER) {
#ifndef VORK_PROPAGATION
    return straight_shadow_transmittance<V,P>(origin,direction,distance,groups,all_casters,transparent,incident_ior VORK_WORK_ARGUMENT);
#else
    let launch_direction=direction;let endpoint=origin+direction*distance;var remaining=distance;var escaped=false;var previous_distance=0.;var result=float3(1.);
    for(uint work=0u;work<REG_PROPAGATION_WORK_LIMIT;work++) {
        float limit;float3 transfer;
        if(!propagation_segment(origin,direction,escaped,previous_distance,limit,transfer)) return float3(0.);
        limit=min(limit,remaining);
        result*=transfer;
        if(occlusion) result*=straight_shadow_transmittance<V,P>(origin,direction,limit,groups,all_casters,transparent,incident_ior VORK_WORK_ARGUMENT);
        if(!any(result>float3(0.))) return float3(0.);
        origin+=direction*limit;remaining-=limit;
        if(remaining<=0. || limit==REG_FP32_MAX) {
            return (distance==REG_FP32_MAX ? length(direction-launch_direction)<=REG_FP32_GAMMA7 : length(origin-endpoint)<=REG_FP32_GAMMA7*max(1.,distance)) ? result : float3(0.);
        }
    }
    path_boundary_error(REG_PATH_PROPAGATION_ERROR,3u);return float3(0.);
#endif
}
float3 origin_offset(uint object,float3 world,float3 normal,float3 direction) {
    return surface_ray_origin(world,surface_position_error(object,world),normal,direction);
}
// The hit's closure through the raster surface evaluation (hit.slang). A mixed
// surface picks one lobe by `u`; `weight` scales its radiance (Cycles' closure pick).
// A material's lit stage replaces it with the closure it returns (lit_surface),
// its shadow rays seeded by `seed`.
VORK_PATH_HELPER
Closure path_closure<V:IVariant,P:IProgram>(DielectricHit hit, float3 direction, float u, uint seed, out float3 geometric, out bool entering, out float weight, out HitSurface shading) {
    // One output pixel in clip units: the camera footprint hits are filtered with.
    uint width,height;path_output.GetDimensions(width,height);
    var h=dielectric_surface<V,P,PathGeometry>(hit,direction,float2(2./float(width),2./float(height)),Lobe(select(P.lobes,LOBE_PICK,0u),u));
    if (P.lit && P.lit_material(h.v)) {h.s=lit_surface<V,PathShadows<V,P>,P>(h.v,h.entering,h.k,h.s,hit_site(h.v,h.k,h.face,uint2(seed,pixel_hash(seed)),uint(frame.temporal.x),true));}
    shading=h;geometric=h.face;entering=h.entering;weight=h.s.weight;
    var c=h.s.closure;
#ifdef VORK_LIGHT_PATHS
    c.base_color=path_spectrum(c.base_color);c.specular_color=path_spectrum(c.specular_color);
    c.transmission_color=path_spectrum(c.transmission_color);c.subsurface_color=path_spectrum(c.subsurface_color);
    c.coat_color=path_spectrum(c.coat_color);c.fuzz_color=path_spectrum(c.fuzz_color);
    c.emission_color=path_spectrum(c.emission_color,true);
    c.specular_ior=spectral_ior(c.specular_ior,path_sensor_context.wavelength*path_wavelength_scale,c.transmission_weight>0. ? c.transmission_dispersion_scale : 0.,c.transmission_dispersion_abbe_number);
#endif

    // Alpha-masked candidates were already rejected during traversal.
    let r=path_words[hit.instance*REG_RAY_SLOT_WORDS+1u];
    return c;
}

// A fiber scatter is one event across its cross-section. Start the outgoing
// ray beyond this strand, preserving blockers from all other strands.
float3 path_origin(HitSurface h,float3 world,float3 error,float3 direction) {
    if (!h.s.is_hair) {
        let reflection=dot(h.s.geometric,direction)>0.;
        return surface_ray_origin(world+select(reflection,surface_shadow_offset(h.k.shadow_offset,h.face,h.s.geometric,direction),float3(0.)),error,
            select(reflection,h.s.geometric,h.face),direction);
    }
    let model=object_model(h.v.object);let inverse=transpose(object_normal(h.v.object));
    let local_p=mul(float4(world,1.),inverse).xyz;
    let outward=h.face*select(h.entering,1.,-1.);
    let local_n=mul(float4(outward,0.),transpose(model)).xyz;
    let local_d=mul(float4(direction,0.),inverse).xyz;
    let offset=strand_shadow(local_p,local_n,local_d,objects[h.v.object].padding.x,h.v.curve_strand);
    return world+mul(float4(offset,0.),model).xyz;
}

// Position-dependent analytic source proposal; both direct strategies divide
// by this same complete mixture mass at the scattering vertex.
uint pick_light(float2 u, float3 world, out float probability) {
    let count=uint(frame_light_count(frame));
    let pick=light_tree_pick(path_light_nodes,count==0u ? 0u : 2u*count-1u,world,u);
    probability=pick.probability;
    return pick.identity;
}
// The shared evaluation's shadows at a path hit (lit stages): one shadow ray per
// light toward a sampled point of it, seeded by `pixel`, as next events trace.
struct PathShadows<V:IVariant,P:IProgram> : IShadows {
    static const bool probe_rays=true;
    static bool probe_visible(float3 world,float3 normal,float3 position_error,float3 probe) {return true;}
    static uint integration_count(uint index,out float3 normalization) {let count=index==0xffffffffu ? sample_count() : 1u;normalization=float3(float(count));return count;}
    static float3 integration_sample(uint tap,inout uint index,inout float3 world) {if(index==0xffffffffu) {index=tap;}return float3(1.);}
    static uint sample_count() {return uint(frame.temporal.y);}
    static float3 visibility_direction(float3 world,float3 normal,LightValues record,uint light,float3 direction,float distance,float3 position_error,uint index,uint count,bool fibre=false) {
        let flags=lights[light].shadow_offsets[REG_LIGHT_RAY_WORD];
        var rng=pixel_hash(index^pixel_hash(light)^pixel_hash(asuint(world.x))^pixel_hash(asuint(world.y))^pixel_hash(asuint(world.z)));
#ifdef VORK_MEDIUM_WORK_LIMIT
        path_boundary_error(REG_PATH_MEDIUM_WORK,0xff000000u);return float3(0.);
#else
        let attenuation=participating_transmittance(world,direction,distance,rng VORK_WORK_ARGUMENT);
#ifndef VORK_PROPAGATION
        if ((flags&REG_LIGHT_RAY_SHADOWED)==0u) {return attenuation;}
#endif
        let origin=surface_ray_origin(world,position_error,normal,direction);
        let connection=surface_ray_connection(origin,world,direction,distance);
        return attenuation*path_shadow_transmittance<V,P>(origin,connection.xyz,connection.w,lights[light].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,0u,1.,(flags&REG_LIGHT_RAY_SHADOWED)!=0u VORK_WORK_ARGUMENT);
#endif
    }
    static const bool subsurface=true;
    static float thickness(uint object,float3 world,float3 direction) {return subsurface_exit(path_scene,object,world,direction);}
};
float3 connection_attenuation(DielectricInterface media,float3 geometric,float3 direction,float distance,float3 world,inout uint rng VORK_WORK_PARAMETER) {
#ifdef VORK_MEDIUM_WORK_LIMIT
    if(!medium_work_reserve(medium_work,1u+path_settings.limits.z,3u,0u)) {return float3(0.);}
#endif
    return segment_attenuation(dot(geometric,direction)<0. ? media.transmitted : media.incident,distance)*participating_transmittance(world,direction,distance,rng VORK_WORK_ARGUMENT);
}
float mis_power(float sampled, float other) {
    if (sampled<=0.) {return 0.;}
    let ratio=other/sampled;return 1./(1.+ratio*ratio);
}
bool path_event_hemisphere(Closure c,uint event,float3 direction,float3 face,float3 smooth) {
    if(event==REG_RAY_TRANSMISSION) {return dot(face,direction)<0. && dot(smooth,direction)<0.;}
    return (event==REG_RAY_DIFFUSE && c.geometry_thin_walled) || dot(smooth,direction)>0.;
}
float3 analytic_mis_response(Closure c, float3 view, float3 direction, float3 geometric, uint contribution, float light_pdf, TransportPreparation prepared) {
    var value=float3(0.);
    for (var i=0u;i<3u;i++) {
        let event=i==0u ? REG_RAY_DIFFUSE : (i==1u ? REG_RAY_SPECULAR : REG_RAY_TRANSMISSION);
        if ((contribution&event)==0u || !path_event_hemisphere(c,event,direction,geometric,prepared.smooth_normal)) {continue;}
        let weight=light_pdf>0. ? mis_power(light_pdf,transport_pdf_mask(c,view,direction,event,prepared)) : 1.;
        value+=transport_eval_mask(c,view,direction,event,prepared)*weight;
    }
    return value;
}
float3 next_event<V:IVariant,P:IProgram>(HitSurface shading, Closure c, float3 view, float3 world, float3 error, float3 geometric, uint groups, uint transparent, DielectricInterface media, float subsurface_thickness, TransportPreparation prepared, BsdfSample sample, PathSampler sampler, float optical_length, inout uint rng VORK_WORK_PARAMETER) {
    let count=uint(frame_light_count(frame));if (count==0u) {return float3(0.);}
    let u=path_light(sampler);
    float probability;let i=pick_light(u.zw,world,probability);if (probability<=0.) {return float3(0.);}
    let contribution=lights[i].shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD];
    if (!light_receives(contribution,lights[i].shadow_offsets[REG_LIGHT_LINK_WORD],groups)) {return float3(0.);}
    let l=light_values<Unspecialized>(i);
    let source=emitter_sample_li<V>(l,i,world,u.xy,uint(frame.temporal.x));let distance=source.distance;let axis=source.direction;
#ifdef VORK_PROPAGATION
    var bsdf_endpoint=float3(0.);
    // Both strategies measure launch solid angle. Delta events retain unit MIS.
    if(!source.delta && any(sample.weight>float3(0.)) && dot(sample.direction,sample.direction)>0.
        && (contribution&sample.event)!=0u
        && (shading.s.is_hair || path_event_hemisphere(c,sample.event,sample.direction,geometric,shading.s.geometric))
        && !(transport_null_event(c,media.incident.ior,sample))) {
        let optical=dot(shading.s.geometric,sample.direction)<0. ? media.transmitted : media.incident;
        let weight=sample.pdf>0. ? mis_power(sample.pdf,segment_light_pdf<V>(l,world,sample.direction)) : 1.;
        bsdf_endpoint=sample.weight*(weight/probability)*segment_analytic<V,P>(path_origin(shading,world,error,sample.direction),sample.direction,i,transparent,optical,optical_length,rng);
    }
    if(!source.delta) {
        float launch_pdf;let launch=segment_light_direction<V>(l,world,source.direction,source.pdf,rng,launch_pdf);
        if(launch_pdf<=0.) return bsdf_endpoint;
        var response=analytic_mis_response(c,view,launch,geometric,contribution,launch_pdf,prepared);
        if(shading.s.is_hair) {
            float fibre_pdf=0.;response=(contribution&REG_RAY_SPECULAR)!=0u ? strand_direct(shading.s.hair0,shading.s.hair1,shading.s.strand,shading.s.geometric,view,launch,fibre_pdf) : float3(0.);
            response*=mis_power(launch_pdf,fibre_pdf);
        }
        if(!any(response>float3(0.))) return bsdf_endpoint;
        let optical=dot(shading.s.geometric,launch)<0. ? media.transmitted : media.incident;
        return bsdf_endpoint+response/(probability*launch_pdf)*segment_analytic<V,P>(path_origin(shading,world,error,launch),launch,i,transparent,optical,optical_length,rng);
    }
#endif
    let light_pdf=source.pdf;
    let power=path_spectrum(source.delta ? source.radiance : (source.pdf>0. ? source.radiance/source.pdf : float3(0.)),true);
    // Hair answers lights through its specular contribution in either hemisphere,
    // without medium attenuation, in both estimators below.
    let hair=shading.s.is_hair;let hair_lit=hair && (contribution&REG_RAY_SPECULAR)!=0u;
    var response=float3(0.);
    if (hair) {
        float fibre_pdf;let fibre=strand_direct(shading.s.hair0,shading.s.hair1,shading.s.strand,shading.s.geometric,view,axis,fibre_pdf);
        if (hair_lit) {response=fibre*(light_pdf>0. ? mis_power(light_pdf,fibre_pdf) : 1.);}
    } else {
        response=analytic_mis_response(c,view,axis,geometric,contribution,light_pdf,prepared);
    }
    var shadow_world=world;var shadow_normal=geometric;var through_boundary=false;
    let original=shading.s.closure;
    if ((frame.integrator.w&REG_INTEGRATOR_SUBSURFACE_THICKNESS)!=0u && thickness_active(original) && !shading.s.is_hair && dot(geometric,axis)<0. && (contribution&REG_RAY_DIFFUSE)!=0u) {
        let thickness=subsurface_thickness;
        let chord=subsurface_exit(path_scene,shading.v.object,world,axis);
        if (thickness>=0. && chord>=0. && chord<distance) {
            let l=layers<Unspecialized>(c,view,prepared.coat_albedo);
            response+=subsurface_transmission(original,geometric,view,axis,thickness)*max(-dot(geometric,axis),0.)/REG_PI*l.throughput*l.scale;
            shadow_world=world+axis*chord;shadow_normal=axis;through_boundary=true;
        }
    }
    var value=response*power/probability*connection_arrival(optical_length,distance,dot(shading.s.geometric,axis)<0. ? media.transmitted.ior : media.incident.ior);
    // Attenuate the connection in its outgoing medium, including a light
    // inside glass reached by transmission at the entry boundary. Thin sheets
    // preserve the current medium on both hemispheres.
    if (!hair) {value*=connection_attenuation(media,shading.s.geometric,axis,distance,world,rng VORK_WORK_ARGUMENT);}
    let threshold=path_settings.adaptive.z;let importance=max(value.x,max(value.y,value.z));
    if (importance<threshold && threshold>0.) {let keep=importance/threshold;if(path_random(rng)>=keep){value=float3(0.);}else{value/=keep;}}
    let flags=lights[i].shadow_offsets[REG_LIGHT_RAY_WORD];
    // Zero in this estimator does not suppress the independent endpoint below.
    if (any(value!=float3(0.)) && ((flags&REG_LIGHT_RAY_SHADOWED)!=0u
#ifdef VORK_PROPAGATION
        || true
#endif
    )) {
        let origin=through_boundary ? origin_offset(shading.v.object,shadow_world,shadow_normal,axis) : path_origin(shading,world,error,axis);
        let connection=surface_ray_connection(origin,world,axis,distance);
        value*=path_shadow_transmittance<V,P>(origin,connection.xyz,connection.w,lights[i].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,transparent,dot(shading.s.geometric,axis)<0. ? media.transmitted.ior : media.incident.ior,(flags&REG_LIGHT_RAY_SHADOWED)!=0u VORK_WORK_ARGUMENT);
    }
#ifdef VORK_PROPAGATION
    return value;
#endif
    // Reuse the ordinary continuation sample as the second direct estimator.
    // This endpoint query is conditional on the selected light: O(log N) light
    // selection, one shape test, no scan or second analytic-light acceleration.
    // It also runs at the last scattering vertex, where NEE is still evaluated.
    let continuation=hair ? hair_lit
        : (contribution&sample.event)!=0u && path_event_hemisphere(c,sample.event,sample.direction,geometric,shading.s.geometric);
    // Null events belong to the preceding connection, not another emitter strategy.
    if (continuation && !(transport_null_event(c,media.incident.ior,sample)) && dot(sample.direction,sample.direction)>0.) {
        let source=emitter_eval_li<V>(l,world,sample.direction);
        if (source.pdf>0.) {
            let weight=sample.pdf>0. ? mis_power(sample.pdf,source.pdf) : 1.;
            var second=sample.weight*path_spectrum(source.radiance,true)*(weight/probability)*connection_arrival(optical_length,source.distance,dot(shading.s.geometric,sample.direction)<0. ? media.transmitted.ior : media.incident.ior);
            if (!hair) {second*=connection_attenuation(media,shading.s.geometric,sample.direction,source.distance,world,rng VORK_WORK_ARGUMENT);}
            let importance=max(second.x,max(second.y,second.z));
            if (importance<threshold && threshold>0.) {let keep=importance/threshold;if(path_random(rng)>=keep){second=float3(0.);}else{second/=keep;}}
            if (any(second!=float3(0.)) && ((flags&REG_LIGHT_RAY_SHADOWED)!=0u)) {
                let origin=path_origin(shading,world,error,sample.direction);
                let connection=surface_ray_connection(origin,world,sample.direction,source.distance);
                second*=path_shadow_transmittance<V,P>(origin,connection.xyz,connection.w,lights[i].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,transparent,dot(shading.s.geometric,sample.direction)<0. ? media.transmitted.ior : media.incident.ior,true VORK_WORK_ARGUMENT);
            }
            value+=second;
        }
    }
    return value;
}
float3 path_environment(float3 direction, uint event, uint groups, float3 origin=float3(0.)) {
    let distant=event==REG_RAY_CAMERA ? camera_distant_lights(frame,lights,direction) : float3(0.);
    if ((frame.environment_links.x&event)==0u || (event!=REG_RAY_CAMERA && !light_receives(frame.environment_links.x,frame.environment_links.y,groups))) {return path_spectrum(distant,true);}
    var radiance=frame.radiance.xyz+distant;
    if (frame.environment.x>0.) {
        var source=environment_radiance(path_environment_source,environment_uv(direction,frame.environment_rotation),uint(frame.environment_color.w),event!=REG_RAY_CAMERA);
        if (event==REG_RAY_CAMERA) {
#ifdef VORK_SPATIAL_ENVIRONMENT
            source=primary_environment(mul(float4(origin,0.),frame.environment_rotation).xyz,normalize(mul(float4(direction,0.),frame.environment_rotation).xyz),frame.scene.x);
#else
#endif
        }
        radiance+=source*frame.environment.x*frame.environment_color.rgb;
    }
    return path_spectrum(radiance,true);
}
bool environment_strategy() {return frame.environment.x>0.;}
float environment_density(float3 direction) {
    let local=normalize(mul(float4(direction,0.),frame.environment_rotation).xyz);
    return environment_proposal_pdf(path_environment_distribution,path_settings.environment.xy,local);
}
float3 environment_direction(float2 u, out float pdf) {
    let local=environment_proposal_sample(path_environment_distribution,path_settings.environment.xy,u,pdf);
    return normalize(mul(frame.environment_rotation,float4(local,0.)).xyz);
}
// Both strategies use the same joint event / solid-angle measure. Scaling by
// the largest density avoids overflow for tiny or nearly edge-on emitters.
uint mesh_emitters() {return asuint(path_settings.adaptive.y);}
float mesh_pdf<V:IVariant>(DielectricHit hit, float3 from, float3 point, float3 face) {
    if(hit.field) {return field_emitter_pdf<PathGeometry>(path_fields<V>(),hit.instance,from);}
    let base=mesh_emitters();
    if (path_words[base]==0u || path_words[base+3u+hit.instance]==0xffffffffu) {return 0.;}
    let area=hit_emitter<PathGeometry>(hit.instance,hit.primitive,hit.object,hit.world_to_object,hit.barycentrics).area;
    return emitter_mesh_pdf<PathGeometry>(base,hit.instance,area,from,point,face)*emitter_mesh_probability<PathGeometry>(base,hit.instance,hit.primitive,path_emitter_weights);
}
uint continuation_mask(uint bounce, uint diffuse, uint glossy, uint transmission) {
    if (bounce>=path_settings.bounces.x) {return 0u;}
    var mask=0u;
    if (diffuse<path_settings.bounces.y) {mask|=REG_RAY_DIFFUSE;}
    if (glossy<path_settings.bounces.z && (diffuse==0u || (path_settings.limits.w&1u)!=0u)) {mask|=REG_RAY_SPECULAR;}
    if (transmission<path_settings.bounces.w && (diffuse==0u || (path_settings.limits.w&2u)!=0u)) {mask|=REG_RAY_TRANSMISSION;}
    return mask;
}
// Same event visibility and alpha/null continuations as the BSDF path. Shadow
// membership would be a different strategy: a diffuse-invisible blocker must
// not hide emission that a diffuse continuation can reach (and vice versa).
float3 mesh_connection<V:IVariant,P:IProgram>(float3 origin, float3 direction, float distance, uint event, uint transparent, float incident_ior, inout uint rng VORK_WORK_PARAMETER) {
#ifdef VORK_MEDIUM_WORK_LIMIT
    if(!medium_work_reserve(medium_work,1u+path_settings.limits.z,3u,0u)) {return float3(0.);}
#endif
    var remaining=distance;var transmittance=float3(1.);
    for (;;) {
        let hit=trace<V,P>(origin,direction,remaining,event VORK_WORK_ARGUMENT);
        if (hit.t<0.) {return transmittance;}
        float3 face;bool entering;float weight;HitSurface shading;
        let c=path_closure<V,P>(hit,direction,P.lobes ? path_random(rng) : 0.,P.lit ? pixel_hash(rng) : 0u,face,entering,weight,shading);
        if (c.geometry_opacity>=1. || path_random(rng)<c.geometry_opacity) {
            transmittance*=shading.s.is_hair ? float3(0.) : transport_null_transmittance(c,-direction,incident_ior,shading.s.geometric,face)*weight;
            if (!any(transmittance>float3(0.))) {return float3(0.);}
        }
        if (transparent++>=path_settings.limits.z) {return float3(0.);}
        let point=shading.v.world;
        let next=origin_offset(hit.object,point,face,direction);
        remaining-=hit.t+length(next-point);origin=next;
        if (remaining<=0.) {return transmittance;}
    }
}
struct ImplicitSourceSample {bool found;uint object;float3 point;float3 face;float3 direction;float distance;float pdf;float3 emitted;};
ImplicitSourceSample implicit_source<V:IVariant,P:IProgram>(float3 world,PathSampler sampler) {
    ImplicitSourceSample result={};let count=path_fields<V>().w;
    if(count==0u) {return result;}
    let u=path_triangle(sampler);
    let selected=field_emitter<PathGeometry>(path_fields<V>(),min(uint(u.x*float(count)),count-1u));
    if(!catcher_keeps(selected.object) || collapsed_endpoint(objects[selected.object].model)) {return result;}
    float density;let direction=field_emitter_direction<PathGeometry>(selected,world,u.yz,density);
    if(density<=0.) {return result;}
    var minimum=0.;
    for(;;) {
        let root=field_intersection<PathGeometry>(selected.descriptor,selected.object,world,direction,minimum,REG_FP32_MAX);
        if(!root.found) {return result;}
        let source=field_surface<V,P>(selected.object,root,direction,float2(0.),Lobe(0u,0.));
        if(!source.s.discarded && source.s.closure.geometry_opacity>0.) {
            if(root.t<=0. || !material_covers_side<V>(source.v,source.entering)) {return result;}
            result.found=true;result.object=selected.object;result.point=source.v.world;
            result.face=source.face;result.direction=direction;result.distance=root.t;
            result.pdf=density/float(count);
            result.emitted=path_spectrum(emitter_mesh_radiance<V,P>(source),true)*source.s.closure.geometry_opacity;
            return result;
        }
        minimum=root.t+REG_FP32_GAMMA7*max(1.,abs(root.t));
    }
}
float3 implicit_next_event<V:IVariant,P:IProgram>(HitSurface receiver,Closure c,float3 view,float3 world,float3 error,float3 geometric,DielectricInterface media,uint continuation,uint transparent,TransportPreparation prepared,PathSampler sampler,float optical_length,inout uint rng VORK_WORK_PARAMETER) {
    if(receiver.s.is_hair || path_fields<V>().w==0u) {return float3(0.);}
    let source=implicit_source<V,P>(world,sampler);
    if(!source.found || !any(source.emitted>float3(0.))) {return float3(0.);}
    let axis=source.direction;let visible=objects[source.object].metadata.y;
    var response=float3(0.);
    for(uint i=0u;i<3u;i++) {
        let event=i==0u ? REG_RAY_DIFFUSE : (i==1u ? REG_RAY_SPECULAR : REG_RAY_TRANSMISSION);
        if((visible&event)==0u || !path_event_hemisphere(c,event,axis,geometric,prepared.smooth_normal)) {continue;}
        let f=transport_eval_mask(c,view,axis,event,prepared);if(!any(f>float3(0.))) {continue;}
        let bsdf_pdf=(continuation&event)!=0u ? transport_pdf_mask(c,view,axis,event,prepared) : 0.;
        let start=path_origin(receiver,world,error,axis);let end=origin_offset(source.object,source.point,source.face,-axis);
        let connection=end-start;let distance=length(connection);
        response+=f*mis_power(source.pdf,bsdf_pdf)*mesh_connection<V,P>(start,connection/distance,distance,event,transparent,dot(prepared.smooth_normal,axis)<0. ? media.transmitted.ior : media.incident.ior,rng VORK_WORK_ARGUMENT);
    }
    return response*source.emitted/source.pdf*connection_attenuation(media,prepared.smooth_normal,axis,source.distance,world,rng VORK_WORK_ARGUMENT)
        *connection_arrival(optical_length,source.distance,dot(prepared.smooth_normal,axis)<0. ? media.transmitted.ior : media.incident.ior);
}

float3 environment_next_event<V:IVariant,P:IProgram>(HitSurface receiver, Closure c,float3 view,float3 world,float3 error,float3 geometric,DielectricInterface media,uint groups,uint continuation,uint transparent,TransportPreparation prepared,PathSampler sampler,inout uint rng VORK_WORK_PARAMETER) {
    if (!environment_strategy() || receiver.s.is_hair) {return float3(0.);}
    float pdf;let direction=environment_direction(path_environment(sampler),pdf);
    if (pdf<=0.) {return float3(0.);}
    var result=float3(0.);
    for (var i=0u;i<3u;i++) {
        let event=i==0u ? REG_RAY_DIFFUSE : (i==1u ? REG_RAY_SPECULAR : REG_RAY_TRANSMISSION);
        if (!path_event_hemisphere(c,event,direction,geometric,prepared.smooth_normal)) {continue;}
        let emitted=path_environment(direction,event,groups);
        if (max(emitted.x,max(emitted.y,emitted.z))<=0.) {continue;}
        let response=transport_eval_mask(c,view,direction,event,prepared);
        if (max(response.x,max(response.y,response.z))<=0.) {continue;}
        let bsdf_pdf=(continuation&event)!=0u ? transport_pdf_mask(c,view,direction,event,prepared) : 0.;
        let start=path_origin(receiver,world,error,direction);
        result+=emitted*response*mis_power(pdf,bsdf_pdf)/pdf
            *mesh_connection<V,P>(start,direction,REG_FP32_MAX,event,transparent,dot(prepared.smooth_normal,direction)<0. ? media.transmitted.ior : media.incident.ior,rng VORK_WORK_ARGUMENT);
    }
    return result*connection_attenuation(media,prepared.smooth_normal,direction,REG_FP32_MAX,world,rng VORK_WORK_ARGUMENT)*path_arrival(REG_FP32_MAX);
}
float3 mesh_next_event<V:IVariant,P:IProgram>(HitSurface receiver, Closure c, float3 view, float3 world, float3 error, float3 geometric, DielectricInterface media, uint continuation, uint transparent, TransportPreparation prepared, PathSampler sampler, float optical_length, inout uint rng VORK_WORK_PARAMETER) {

    let base=mesh_emitters();let count=path_words[base];
    // The fiber sampler has no solid-angle density yet; keep its existing
    // emitter-hit estimator until it can participate in the same contract.
    if (count==0u || receiver.s.is_hair) {return float3(0.);}
    let selected=emitter_mesh_sample<PathGeometry>(base,path_triangle(sampler),path_emitter_weights);
    if (!catcher_keeps(selected.object)) {return float3(0.);}
    DielectricHit hit={};hit.instance=selected.instance;hit.object=selected.object;hit.primitive=selected.primitive;
    hit.world_to_object=(float3x4)object_normal(hit.object);hit.barycentrics=selected.barycentrics;
    let geometry=hit_emitter<PathGeometry>(hit.instance,hit.primitive,hit.object,hit.world_to_object,hit.barycentrics);
    let point=geometry.point;let delta=point-world;let distance=length(delta);
    if (distance<=0.) {return float3(0.);}
    let axis=delta/distance;let source_face=geometry.face;
    let pdf=mesh_pdf<V>(hit,world,point,source_face);if (pdf<=0.) {return float3(0.);}
    let source=dielectric_surface<V,P,PathGeometry>(hit,axis,float2(0.),Lobe(0u,0.));
    if(source.s.discarded) {return float3(0.);}
    let emitted=path_spectrum(emitter_mesh_radiance<V,P>(source),true)*source.s.closure.geometry_opacity;
    if (max(emitted.x,max(emitted.y,emitted.z))<=0.) {return float3(0.);}
    let visible=objects[hit.object].metadata.y;
    var response=float3(0.);
    for (var i=0u;i<3u;i++) {
        let event=i==0u ? REG_RAY_DIFFUSE : (i==1u ? REG_RAY_SPECULAR : REG_RAY_TRANSMISSION);
        if ((visible&event)==0u || !path_event_hemisphere(c,event,axis,geometric,prepared.smooth_normal)) {continue;}
        let f=transport_eval_mask(c,view,axis,event,prepared);if (max(f.x,max(f.y,f.z))<=0.) {continue;}
        let bsdf_pdf=(continuation&event)!=0u ? transport_pdf_mask(c,view,axis,event,prepared) : 0.;
        let start=path_origin(receiver,world,error,axis);let end=origin_offset(hit.object,point,source.face,-axis);
        let connection=end-start;let length_to_source=length(connection);
        response+=f*mis_power(pdf,bsdf_pdf)*mesh_connection<V,P>(start,connection/length_to_source,length_to_source,event,transparent,dot(prepared.smooth_normal,axis)<0. ? media.transmitted.ior : media.incident.ior,rng VORK_WORK_ARGUMENT);
    }
    return response*emitted/pdf*connection_attenuation(media,prepared.smooth_normal,axis,distance,world,rng VORK_WORK_ARGUMENT)
        *connection_arrival(optical_length,distance,dot(prepared.smooth_normal,axis)<0. ? media.transmitted.ior : media.incident.ior);
}

#ifdef VORK_PATH_MEDIA
float3 phase_connection<V:IVariant,P:IProgram>(float3 world,float3 direction,float distance,uint transparent,PathBoundary optical,inout uint rng VORK_WORK_PARAMETER) {
#ifdef VORK_MEDIUM_WORK_LIMIT
    if(!medium_work_reserve(medium_work,1u+path_settings.limits.z,3u,0u)) {return float3(0.);}
#endif
    var value=float3(1.);
    value*=mesh_connection<V,P>(world,direction,distance,REG_RAY_VOLUME,transparent,optical.ior,rng VORK_WORK_ARGUMENT);
    value*=participating_transmittance(world,direction,distance,rng VORK_WORK_ARGUMENT);
    return value*boundary_attenuation(optical,distance);
}
float3 phase_analytic<V:IVariant,P:IProgram>(PathMediumInterval medium,PathPhaseDirection phase,float3 incident,float3 world,uint groups,uint transparent,PathBoundary optical,PathSampler sampler,float optical_length, inout uint rng VORK_WORK_PARAMETER) {
    if(uint(frame_light_count(frame))==0u) {return float3(0.);}
    let u=path_light(sampler);
    float probability;let i=pick_light(u.zw,world,probability);if(probability<=0.) {return float3(0.);}
    let l=light_values<Unspecialized>(i);let contribution=lights[i].shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD];
    if((contribution&REG_RAY_VOLUME)==0u) {return float3(0.);}
    let source=emitter_sample_li<V>(l,i,world,u.xy,uint(frame.temporal.x));var value=float3(0.);
    let flags=lights[i].shadow_offsets[REG_LIGHT_RAY_WORD];
    let shadowed=(flags&REG_LIGHT_RAY_SHADOWED)!=0u;
    if(source.delta
#ifndef VORK_PROPAGATION
        || source.pdf>0.
#endif
    ) {
        let cosine=dot(incident,source.direction);
        let weight=source.delta ? 1. : mis_power(source.pdf,medium.pdf(cosine))/source.pdf;
        var connection=participating_transmittance(world,source.direction,source.distance,rng VORK_WORK_ARGUMENT)*boundary_attenuation(optical,source.distance);
        if(shadowed
#ifdef VORK_PROPAGATION
            || true
#endif
        ) {connection*=path_shadow_transmittance<V,P>(world,source.direction,source.distance,lights[i].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,transparent,optical.ior,shadowed VORK_WORK_ARGUMENT);}
        value+=medium.linked_phase(cosine,contribution,lights[i].shadow_offsets[REG_LIGHT_LINK_WORD])*path_spectrum(source.radiance,true)*connection*weight*connection_arrival(optical_length,source.distance,optical.ior);
    }
#ifdef VORK_PROPAGATION
    if(!source.delta) {
        float launch_pdf;let launch=segment_light_direction<V>(l,world,source.direction,source.pdf,rng,launch_pdf);
        if(launch_pdf>0.) value+=medium.linked_phase(dot(incident,launch),contribution,lights[i].shadow_offsets[REG_LIGHT_LINK_WORD])*(mis_power(launch_pdf,medium.pdf(dot(incident,launch)))/launch_pdf)
            *segment_analytic<V,P>(world,launch,i,transparent,path_segment(optical),optical_length,rng);
    }
    if(!source.delta && phase.measure>0.) {
        let weight=phase.pdf>0. ? mis_power(phase.pdf,segment_light_pdf<V>(l,world,phase.direction)) : 1.;
        value+=medium.linked_phase(phase.cosine,contribution,lights[i].shadow_offsets[REG_LIGHT_LINK_WORD],phase.pdf==0.)*(weight/phase.measure)
            *segment_analytic<V,P>(world,phase.direction,i,transparent,path_segment(optical),optical_length,rng);
    }
#else
    let endpoint=emitter_eval_li<V>(l,world,phase.direction);
    if(endpoint.pdf>0.) {
        var connection=participating_transmittance(world,phase.direction,endpoint.distance,rng VORK_WORK_ARGUMENT)*boundary_attenuation(optical,endpoint.distance);
        if(shadowed) {connection*=path_shadow_transmittance<V,P>(world,phase.direction,endpoint.distance,lights[i].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],(flags&REG_LIGHT_RAY_SHADOWED_BY_ALL)!=0u,transparent,optical.ior,true VORK_WORK_ARGUMENT);}
        value+=medium.linked_phase(phase.cosine,contribution,lights[i].shadow_offsets[REG_LIGHT_LINK_WORD],phase.pdf==0.)/phase.measure*path_spectrum(endpoint.radiance,true)*connection*(phase.pdf>0. ? mis_power(phase.pdf,endpoint.pdf) : 1.)*connection_arrival(optical_length,endpoint.distance,optical.ior);
    }
#endif
    return value/probability;
}
float3 phase_mesh<V:IVariant,P:IProgram>(PathMediumInterval medium,float3 incident,float3 world,bool continuation,uint transparent,PathBoundary optical,PathSampler sampler,float optical_length, inout uint rng VORK_WORK_PARAMETER) {
    let base=mesh_emitters();if(path_words[base]==0u) {return float3(0.);}
    let candidate=emitter_mesh_sample<PathGeometry>(base,path_triangle(sampler),path_emitter_weights);
    if((objects[candidate.object].metadata.y&REG_RAY_VOLUME)==0u) {return float3(0.);}
    let object=object_model(candidate.object);let inverse=transpose(object_normal(candidate.object));
    let geometry=hit_emitter<PathGeometry>(candidate.instance,candidate.primitive,candidate.object,(float3x4)transpose(inverse),candidate.barycentrics);
    let point=geometry.point;let delta=point-world;let distance=length(delta);if(distance<=0.) {return float3(0.);}
    let axis=delta/distance;var face=geometry.face;
    if (!catcher_keeps(candidate.object)) {return float3(0.);}
    DielectricHit hit={};hit.instance=candidate.instance;hit.object=candidate.object;hit.primitive=candidate.primitive;hit.barycentrics=candidate.barycentrics;hit.world_to_object=(float3x4)transpose(inverse);
    let pdf=emitter_mesh_pdf<PathGeometry>(base,hit.instance,geometry.area,world,point,face)*candidate.probability;
    if(pdf<=0.) {return float3(0.);}
    let source=dielectric_surface<V,P,PathGeometry>(hit,axis,float2(0.),Lobe(0u,0.));
    if(source.s.discarded) {return float3(0.);}
    let emitted=path_spectrum(emitter_mesh_radiance<V,P>(source),true)*source.s.closure.geometry_opacity;
    let end=origin_offset(hit.object,point,source.face,-axis);let connection=end-world;let length_to_source=length(connection);
    let cosine=dot(incident,axis);let phase_pdf=continuation ? medium.pdf(cosine) : 0.;
    return medium.phase(cosine)*emitted*(mis_power(pdf,phase_pdf)/pdf)
        *phase_connection<V,P>(world,connection/length_to_source,length_to_source,transparent,optical,rng VORK_WORK_ARGUMENT)
        *connection_arrival(optical_length,distance,optical.ior);
}
float3 phase_implicit<V:IVariant,P:IProgram>(PathMediumInterval medium,float3 incident,float3 world,bool continuation,uint transparent,PathBoundary optical,PathSampler sampler,float optical_length,inout uint rng VORK_WORK_PARAMETER) {
    let source=implicit_source<V,P>(world,sampler);
    if(!source.found || (objects[source.object].metadata.y&REG_RAY_VOLUME)==0u || !any(source.emitted>float3(0.))) {return float3(0.);}
    let end=origin_offset(source.object,source.point,source.face,-source.direction);
    let connection=end-world;let distance=length(connection);
    let cosine=dot(incident,source.direction);let pdf=continuation ? medium.pdf(cosine) : 0.;
    return medium.phase(cosine)*source.emitted*(mis_power(source.pdf,pdf)/source.pdf)
        *phase_connection<V,P>(world,connection/distance,distance,transparent,optical,rng VORK_WORK_ARGUMENT)
        *connection_arrival(optical_length,source.distance,optical.ior);
}
float3 phase_environment<V:IVariant,P:IProgram>(PathMediumInterval medium,float3 incident,float3 world,bool continuation,uint transparent,PathBoundary optical,PathSampler sampler,inout uint rng VORK_WORK_PARAMETER) {
    if((frame.environment_links.x&REG_RAY_VOLUME)==0u) {return float3(0.);}
    float pdf;let direction=environment_direction(path_environment(sampler),pdf);
    if(pdf<=0.) {return float3(0.);}
    let emitted=path_environment(direction,REG_RAY_VOLUME,medium.receiver_groups);
    let response=medium.linked_phase(dot(incident,direction),frame.environment_links.x,frame.environment_links.y);
    if(!any(emitted>float3(0.)) || !any(response>float3(0.))) {return float3(0.);}
    let phase_pdf=continuation ? medium.pdf(dot(incident,direction)) : 0.;
    let connection=phase_connection<V,P>(world,direction,REG_FP32_MAX,transparent,optical,rng VORK_WORK_ARGUMENT);
    return response*emitted*connection*(mis_power(pdf,phase_pdf)/pdf)*path_arrival(REG_FP32_MAX);
}
#endif

#ifdef VORK_PATH_MEDIA
// A medium can scatter beauty away before its first surface. Camera data passes
// resume the geometric camera segment from that point, with the opacity decisions
// already made and their remaining budget, without consuming beauty's sequence.
void camera_surface_aovs<V:IVariant,P:IProgram>(float3 origin,float3 direction,uint seed,uint remaining,bool escaped,float previous_distance,float pending,out float4 aovs[VORK_AOV_SLOTS],out float4 position,out float4 normal VORK_WORK_PARAMETER) {
    aovs={};position=float4(0.);normal=float4(0.);var rng=seed;var transparent=0u;
    for(uint work=0u;work<REG_PROPAGATION_WORK_LIMIT;work++) {
        if(pending<=0.) {
            float3 transfer;
            if(!propagation_segment(origin,direction,escaped,previous_distance,pending,transfer)) return;
        }
        let hit=trace<V,P>(origin,direction,pending,REG_RAY_CAMERA VORK_WORK_ARGUMENT);
        if(hit.t<0.) {
            if(pending==REG_FP32_MAX) return;
            origin+=direction*pending;pending=0.;continue;
        }
        var u=0.;if(P.lobes) u=path_random(rng);
        float3 geometric;bool entering;float weight;HitSurface shading;
        let c=path_closure<V,P>(hit,direction,u,P.lit ? pixel_hash(rng) : 0u,geometric,entering,weight,shading);
        if(c.geometry_opacity>=1. || path_random(rng)<c.geometry_opacity) {
            aovs=shading.s.aovs;position=float4(shading.v.world,1.);
            normal=float4(normalize(shading.v.NORMAL),objects[hit.object].opacity.y);return;
        }
        if(transparent++>=remaining) return;
        origin=origin_offset(hit.object,shading.v.world,geometric,direction);
        pending=max(0.,pending-hit.t-length(origin-shading.v.world));
    }
#ifdef VORK_PROPAGATION
    path_boundary_error(REG_PATH_PROPAGATION_ERROR,3u);
#endif
}
#endif
#include "path_segment_connection.slang"
#ifdef VORK_NO_BOUNDARY_TRANSACTIONS
static const bool path_boundary_transactions=false;
#else
static const bool path_boundary_transactions=true;
#endif
float3 integrate<let spatial_subsurface:bool,V:IVariant,P:IProgram,let N:uint>(float3 origin, float3 direction, float3 camera_origin, float camera_min, float camera_max, inout PathOccupancy<N> occupancy, PathSampler camera_sampler, inout uint rng, out float alpha, out float4 aovs[VORK_AOV_SLOTS], out float4 guide_albedo, out float4 guide_normal, out uint camera_flags,out float4 primary_position,out float4 primary_normal VORK_WORK_PARAMETER) {
    camera_flags=0u;primary_position=float4(0.);primary_normal=float4(0.);
    aovs={};guide_albedo=float4(0.);guide_normal=float4(0.);
    let camera_ior=occupancy.ior();
    var optical_length=length(origin-camera_origin)*camera_ior;
    var physical_origin=origin;
    uint dropped_light_samples=0u;
    var throughput=float3(1.);var radiance=float3(0.);var event=REG_RAY_CAMERA;var groups=0u;
    // Ray permissions follow BSDF crossings independently of the retained MIS vertex.
    var ray_event=REG_RAY_CAMERA;
    var previous_world=origin;var previous_pdf=0.;
    var diffuse=0u;var glossy=0u;var transmission=0u;var transparent=0u;alpha=1.;
#ifdef VORK_PATH_MEDIA
    var volume=0u;var previous_environment_weight=float3(1.);
#endif
    var sampler=camera_sampler;
    var pending_segment=0.;var preserve_segment=false;
    var escaped=false;var previous_distance=0.;var work=0u;
    for(var bounce=0u;bounce<=path_settings.bounces.x;sampler.vertex++) {
#ifdef VORK_MEDIUM_WORK_LIMIT
        if(!medium_work_reserve(medium_work,1u+path_settings.limits.z,4u,0u)) {break;}
#endif
        if(!preserve_segment) {pending_segment=0.;escaped=false;previous_distance=0.;work=0u;}
        preserve_segment=false;var absorbed=false;
        DielectricHit hit={};hit.t=-1.;var visibility_minimum=0.;
#ifdef VORK_PATH_MEDIA
        float distance;PathMediumInterval medium;var medium_hit=false;
        let before_medium=throughput;let before_emission=radiance;
#endif
        for(;;) {
#ifdef VORK_PROPAGATION
            if(work++>=REG_PROPAGATION_WORK_LIMIT) {path_boundary_error(REG_PATH_PROPAGATION_ERROR,3u);absorbed=true;break;}
#endif
            if(pending_segment<=0.) {
                float3 transfer;
                if(!propagation_segment(origin,direction,escaped,previous_distance,pending_segment,transfer)) {absorbed=true;break;}
                throughput*=transfer;
            }
            let segment_limit=pending_segment;
            let trace_limit=ray_event==REG_RAY_CAMERA ? min(segment_limit,max(0.,camera_max-length(origin-camera_origin))) : segment_limit;
#ifdef VORK_PROPAGATION
            var minimum=0.;
            if(ray_event==REG_RAY_CAMERA) {
                let remaining_clip=camera_min-length(origin-camera_origin);
                minimum=min(segment_limit,max(0.,remaining_clip));
            }
            visibility_minimum=minimum;hit.t=-1.;
            if(minimum<segment_limit) {
                hit=trace<V,P>(origin+direction*minimum,direction,max(0.,trace_limit-minimum),ray_event VORK_WORK_ARGUMENT);
                if(hit.t>=0.) hit.t+=minimum;
            }
#else
            hit=trace<V,P>(origin,direction,trace_limit,ray_event VORK_WORK_ARGUMENT);
#endif
#ifdef VORK_PATH_MEDIA
            medium_hit=false;
            if(visibility_minimum<segment_limit) {
                let visible_origin=origin+direction*visibility_minimum;
                medium_hit=advance_medium(visible_origin,direction,(hit.t<0. ? segment_limit : hit.t)-visibility_minimum,occupancy.current(),ray_event==REG_RAY_CAMERA,optical_length+dot(visible_origin-physical_origin,direction)*occupancy.ior(),throughput,radiance,rng,distance,medium VORK_WORK_ARGUMENT);
                distance+=visibility_minimum;
            }
            if(medium_hit) break;
#endif
            if(hit.t>=0.) {pending_segment=max(0.,pending_segment-hit.t);break;}
            if(segment_limit==REG_FP32_MAX) break;
            pending_segment=0.;
#ifndef VORK_PATH_MEDIA
            throughput*=occupancy.attenuation(segment_limit-visibility_minimum);
#endif
            origin+=direction*segment_limit;
            optical_length+=length(origin-physical_origin)*occupancy.ior();physical_origin=origin;
        }
        if(absorbed) break;
#ifdef VORK_PATH_MEDIA
        if(bounce==0u && (medium_hit || any(throughput!=before_medium) || any(radiance!=before_emission))) {guide_albedo.w=1.;guide_normal.w=-1.;}
        if(medium_hit) {
            let world=origin+direction*distance;
            optical_length+=length(world-physical_origin)*occupancy.ior();physical_origin=world;
            if (bounce==0u && (path_settings.environment.z>0u
#ifdef VORK_PROPAGATION
                || true
#endif
            )) {
                camera_surface_aovs<V,P>(world,direction,rng,path_settings.limits.z-transparent,escaped,previous_distance,max(0.,pending_segment-distance),aovs,primary_position,primary_normal VORK_WORK_ARGUMENT);
            }
            let phase=phase_direction(medium,direction,path_scatter(sampler).xyz);
            let continuation=volume<path_settings.limits.x && bounce<path_settings.bounces.x;
            let optical=occupancy.current();
#ifdef VORK_PROPAGATION
            let axis=segment_direction(path_environment(sampler));let pdf=1./(4.*REG_PI);
            let phase_pdf=continuation ? medium.pdf(dot(direction,axis)) : 0.;
            var direct=phase_analytic<V,P>(medium,phase,direction,world,groups,transparent,optical,sampler,optical_length,rng VORK_WORK_ARGUMENT);
            for(uint component=0u;component<VORK_PATH_MEDIUM_BOUNDS+1u;component++) {
                let m=medium.components[component];
                let response=m.scattering*medium_phase(m.anisotropy,dot(direction,axis));
                if(any(response>float3(0.))) direct+=response/pdf*segment_connection<V,P>(world,axis,REG_RAY_VOLUME,medium.component_groups[component],transparent,path_segment(optical),optical_length,mis_power(pdf,phase_pdf),rng);
            }
#else
            var direct=phase_analytic<V,P>(medium,phase,direction,world,groups,transparent,optical,sampler,optical_length,rng VORK_WORK_ARGUMENT);
            direct+=phase_mesh<V,P>(medium,direction,world,continuation,transparent,optical,sampler,optical_length,rng VORK_WORK_ARGUMENT);
            direct+=phase_implicit<V,P>(medium,direction,world,continuation,transparent,optical,sampler,optical_length,rng VORK_WORK_ARGUMENT);
            direct+=phase_environment<V,P>(medium,direction,world,continuation,transparent,optical,sampler,rng VORK_WORK_ARGUMENT);
#endif
            radiance+=clamp_sample(throughput*direct,bounce==0u ? path_settings.clamp.x : path_settings.clamp.y,dropped_light_samples);
            if(!continuation) {break;}
            throughput*=phase.weight;
            if(!all(isfinite(throughput))) {path_boundary_error(REG_RADIANCE_NONFINITE,hit.object);break;}
            if(max(throughput.x,max(throughput.y,throughput.z))<=0.) {break;}
            if(bounce>=REG_PATH_ROULETTE_START) {
                let survive=path_survival(throughput,N>0u ? occupancy.ior()/camera_ior : 1.);
                if(path_roulette(sampler)>survive) {break;}throughput/=survive;
            }
            let full_phase=phase.pdf>0. ? medium.phase(phase.cosine) : phase.weight*phase.measure;
            let linked_phase=medium.linked_phase(phase.cosine,frame.environment_links.x,frame.environment_links.y,phase.pdf==0.);
            previous_environment_weight=linked_phase/select(full_phase>float3(0.),full_phase,float3(1.));
            groups=medium.receiver_groups;
            previous_world=world;previous_pdf=phase.pdf;
            origin=world;direction=phase.direction;event=REG_RAY_VOLUME;ray_event=event;volume++;bounce++;continue;
        }
#endif
        if (hit.t<0.) {
            // Camera background eligibility follows ray permissions, not the MIS bounce.
            if (ray_event==REG_RAY_CAMERA && frame.flags.z>0.) {alpha=0.;}
            let sky=select(ray_event==REG_RAY_CAMERA && frame.flags.z>0.,float3(0.),select(ray_event==REG_RAY_CAMERA && frame.environment.w==0.,path_spectrum(frame.background.rgb,true),path_environment(direction,ray_event,groups,camera_origin)));
#ifdef VORK_PROPAGATION
            let environment_mis=previous_pdf>0. ? mis_power(previous_pdf,1./(4.*REG_PI)) : 1.;
#else
            let environment_mis=(environment_strategy() || event==REG_RAY_VOLUME) && previous_pdf>0. ? mis_power(previous_pdf,environment_density(direction)) : 1.;
#endif
#ifdef VORK_PATH_MEDIA
            let environment_weight=event==REG_RAY_VOLUME ? previous_environment_weight : float3(1.);
#else
            let environment_weight=float3(1.);
#endif
            radiance+=clamp_sample(throughput*sky*environment_weight*environment_mis*connection_arrival(0.,REG_FP32_MAX,1.),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);break;
        }
        var world=origin+hit.t*direction;float3 geometric;bool entering;float weight;
        let scatter=path_scatter(sampler);
        var u=0.;if (P.lobes) {u=scatter.w;}
        var seed=0u;if (P.lit) {seed=pixel_hash(rng+sampler.vertex);}
        HitSurface shading;
        var c=path_closure<V,P>(hit,direction,u,seed,geometric,entering,weight,shading);world=shading.v.world;
        optical_length+=length(world-physical_origin)*occupancy.ior();physical_origin=world;
        if (N>0u && occupancy.count>0u) {reconcile_occupancy<PathGeometry,N>(occupancy,world,instance_solid<PathGeometry>(hit.instance,path_settings.boundaries.xw),path_settings.boundaries.xw);}
        // One bound for this reconstructed vertex. Every outgoing direction
        // still chooses its own signed offset and outward ULP rounding.
        var error=surface_position_error(hit.object,world);
        var subsurface_thickness=-1.;
        if ((frame.integrator.w&REG_INTEGRATOR_SUBSURFACE_THICKNESS)!=0u && thickness_active(c) && !shading.s.is_hair) {
            subsurface_thickness=subsurface_exit(path_scene,hit.object,world,-geometric);
            if (subsurface_thickness>=0.) {c.subsurface_color=subsurface_slab(c,subsurface_thickness).reflection;}
        }
#ifndef VORK_PATH_MEDIA
        throughput*=occupancy.attenuation(hit.t-visibility_minimum);
#endif
        let solid=N>0u && !c.geometry_thin_walled && c.transmission_weight>0.;
        var boundary=solid ? boundary_descriptor<V,P,PathGeometry>(hit,direction,shading,path_settings.boundaries.xw,float2(2.)/float2(path_image_size())) : path_boundary(0xffffffffu,c);
        // Resolve the interface once; emitter connections only need its segments.
        let incident=occupancy.current();
        BoundaryEvent<N> boundary_changes={};
        if(path_boundary_transactions && solid) boundary_changes=boundary_event<V,P,PathGeometry,N>(path_scene,origin,direction,hit,world,error,geometric,boundary,entering,occupancy,path_settings.boundaries.xw,float2(2.)/float2(path_image_size()),ray_event VORK_WORK_ARGUMENT);
#ifdef VORK_LIGHT_PATHS
        if(!path_boundary_transactions && solid) {
            boundary.color=path_spectrum(boundary.color);
            boundary.ior=spectral_ior(boundary.ior,vork_path_wavelength,boundary.dispersion_scale,boundary.dispersion_abbe);
        }
        for(uint i=0u;i<boundary_changes.count;i++) {
            if(boundary_changes.kind_at(i)!=2u) continue;
            let slot=occupancy.find(boundary_changes.object_at(i));var b=occupancy.at(slot);
            b.color=path_spectrum(b.color);
            b.ior=spectral_ior(b.ior,vork_path_wavelength,b.dispersion_scale,b.dispersion_abbe);
            occupancy.set(slot,b);
        }
#endif
        var transmitted=incident;var owner_neutral=false;
        if(solid) {
            if(path_boundary_transactions) {
                transmitted=boundary_changes.owner(occupancy);
                owner_neutral=incident.object==transmitted.object && incident.medium==transmitted.medium;
            } else transmitted=occupancy.plain_transmitted(boundary,entering,owner_neutral);
        }
        if(solid && owner_neutral) {
            // A neutral crossing is a null event: bound it like every other continuation.
            if (transparent++>=path_settings.limits.z) {break;}
            if(path_boundary_transactions) boundary_changes.finish(occupancy,true);
            else plain_path_cross(occupancy,boundary,entering);
            origin=path_origin(shading,world,error,direction);
            pending_segment=max(0.,pending_segment-length(origin-world));preserve_segment=true;continue;
        }
        let media=path_interface(incident,transmitted);
        let subsurface_ior=c.specular_ior; // absolute phase index, before interface-relative BSDF conversion
        if (N>0u) {
            c=dielectric_interface(c,incident.ior,solid ? transmitted.ior : c.specular_ior);
        }
        if (c.geometry_opacity<1. && path_opacity(sampler)>=c.geometry_opacity) {
            if (transparent++>=path_settings.limits.z) {break;}
            if(path_boundary_transactions && solid) boundary_changes.finish(occupancy,false);
            origin=surface_ray_origin(world,error,geometric,direction);
            pending_segment=max(0.,pending_segment-length(origin-world));preserve_segment=true;continue;
        }
        if (ray_event==REG_RAY_CAMERA) {
            primary_position=float4(world,1.);
            primary_normal=float4(normalize(shading.v.NORMAL),objects[hit.object].opacity.y);
            let flags=objects[hit.object].metadata.y;
            camera_flags=REG_RAY_CAMERA;
            if ((flags&REG_OBJECT_HOLDOUT)!=0u && !catcher_without_cg) {
                if(path_boundary_transactions && solid) boundary_changes.finish(occupancy,false);
                camera_flags=0u;alpha=frame.flags.z>0. ? 0. : 1.;radiance=float3(0.);break;
            }
            if ((flags&REG_OBJECT_CATCHER)!=0u) {camera_flags=REG_OBJECT_CATCHER;}
        }
        // The accepted camera hit already includes mask normalization and
        // stochastic opacity continuation; use the same surface as beauty.
        if(bounce==0u) {
            aovs=shading.s.aovs;
            if(path_settings.environment.w!=0u) {
                // Guides describe the RGB material, not the sampled wavelength.
                let guide_layers=layers<V>(shading.s.closure,-direction);float3 guide_scattering;
                let guide_specular=specular_response<V>(guide_layers.base,-direction);
                let guide_uv=material_uv(shading.v,shading.k.uv_dx,shading.k.uv_dy,shading.k.uv1_dx,shading.k.uv1_dy,shading.k.material_filter);
                let response=surface_terms_at<V,PathShadows<V,P>>(shading.v,shading.s,guide_layers,guide_specular,guide_uv,guide_scattering);
                // Hair has no diffuse/reflected surface-response lobes. Its
                // first-hit albedo is the integral of the Chiang scattering
                // orders, already provided by the uniform-world evaluator.
                let base=guide_layers.base;
                // Dielectric albedo includes transmitted energy. Preserve coat
                // absorption through the existing layer throughput and scale.
                var transmitted=float3(0.);
                if(base.transmission_weight>0. && base.base_metalness<1.) {
                    let moments=brdf_moments(float2(max(dot(base.geometry_normal,-direction),DENOISE_GUIDE_COSINE_MIN),specular_roughness(base.specular_roughness)));
                    let reflected=all(guide_specular.directional_albedo>=float3(0.)) ? guide_specular.directional_albedo : specular_reflectance(guide_specular,moments,internal_reflection_quadrature<V>(base));
                    transmitted=guide_layers.throughput*guide_layers.scale*(float3(1.)-reflected)*base.transmission_weight*(1.-base.base_metalness);
                }
                let albedo=shading.s.is_hair ? strand_world(shading.s.hair0,shading.s.hair1,shading.s.strand,shading.s.geometric,-direction,float3(1.)) : response.diffuse+response.reflected+transmitted;
                guide_albedo=float4(clamp(albedo*weight,float3(0.),float3(1.)),1.);
                let protect=guide_normal.w<0. || reconstruction_protected(shading.s,response);
                guide_normal=float4(response.normal,protect ? -1. : float(hit.object+1u));
            }
        }
        let incident_groups=groups;
        groups=objects[hit.object].metadata.x;
        let emission_throughput=throughput;
        if (P.lobes) {throughput*=weight;}
        // Preserve the authored BSDF after every event. Changing roughness here
        // biases both connections and continuation, with no matching estimator.
#ifdef VORK_PROPAGATION
        let emission_mis=previous_pdf>0. ? mis_power(previous_pdf,1./(4.*REG_PI)) : 1.;
#else
        let emission_mis=previous_pdf>0. ? mis_power(previous_pdf,mesh_pdf<V>(hit,previous_world,world,geometric)) : 1.;
#endif
        // Lit can encode reflected receiver response in an unlit closure. It
        // remains a direction-only term, independent of source-emission MIS.
        if (P.lit && P.lit_material(shading.v)) {
            radiance+=clamp_sample(throughput*c.emission_color*c.emission_luminance*path_arrival(optical_length),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        }
        let emitted=(material_covers_side<V>(shading.v,shading.entering) ? path_spectrum(emitter_mesh_radiance<V,P>(shading),true) : float3(0.));
        radiance+=clamp_sample(emission_throughput*emitted*emission_mis*path_arrival(optical_length),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        if (spatial_subsurface && entering && c.base_weight>0. && thickness_active(c) && !shading.s.is_hair) {
            let amount=c.base_weight*c.subsurface_weight*(1.-c.base_metalness)*(1.-c.transmission_weight);
            let reflection=c.specular_weight*lerp(fresnel(max(dot(c.geometry_normal,-direction),0.),c.specular_ior)
                *max(dot(c.specular_color,REG_LUMINANCE),0.),1.,c.base_metalness);
            let coat=c.coat_weight*fresnel(max(dot(c.geometry_coat_normal,-direction),0.),c.coat_ior);
            let local=c.base_weight*dot(c.base_color,REG_LUMINANCE)*(1.-c.subsurface_weight)*(1.-c.base_metalness)*(1.-c.transmission_weight)
                +reflection+coat+c.fuzz_weight+c.transmission_weight;
            let probability=amount/(amount+local);
            if (probability==1. || path_subsurface_entry(sampler)<probability) {
                let layer=layers<Unspecialized>(c,-direction);
                let reflected=c.specular_weight*c.specular_color*dielectric_albedo(max(dot(c.geometry_normal,-direction),0.),c.specular_roughness,c.specular_ior);
                let entry_weight=amount*layer.scale*layer.throughput*(float3(1.)-reflected)/probability;
                let walk=subsurface_walk<SubsurfaceSegments>(path_scene,path_words,path_settings.boundaries.y,path_words[path_settings.boundaries.y+hit.instance],world,geometric,c,rng);
                if (max(walk.weight.x,max(walk.weight.y,walk.weight.z))<=0.) {if(path_boundary_transactions && solid) boundary_changes.finish(occupancy,false);break;}
                optical_length+=walk.length*subsurface_ior;
                throughput*=entry_weight*walk.weight;
                if (!walk.local) {
                    DielectricHit exit={};exit.t=walk.boundary.t;exit.object=walk.boundary.object;
                    exit.instance=walk.boundary.instance;exit.primitive=walk.boundary.primitive;
                    exit.barycentrics=walk.boundary.barycentrics;exit.world_to_object=walk.boundary.world_to_object;
                    float unused_weight;bool unused_entering;
                    // Reverse the interior ray for the exit's outward shading frame.
                    let unused=path_closure<V,P>(exit,-walk.direction,0.,seed,geometric,unused_entering,unused_weight,shading);
                    world=shading.v.world;error=surface_position_error(exit.object,world);groups=objects[exit.object].metadata.x;
                }
                physical_origin=world;
                let exit_normal=shading.s.closure.geometry_normal;
                c=Closure();c.base_color=float3(1.);c.base_weight=1.;c.specular_weight=0.;
                c.specular_ior=1.;c.specular_color=float3(1.);c.specular_roughness=.5;c.geometry_opacity=1.;c.coat_ior=1.5;
                c.geometry_normal=exit_normal;c.geometry_coat_normal=exit_normal;
                c.subsurface_weight=0.;c.transmission_weight=0.;c.fuzz_weight=0.;c.coat_weight=0.;
                shading.s.closure=c;shading.s.is_hair=false;direction=-geometric;subsurface_thickness=-1.;
            } else {
                throughput/=1.-probability;
                // Remove only the spatial substrate, preserving the metal's
                // base-colour Fresnel and the existing specular/coat/fuzz lobes.
                c.subsurface_color=float3(0.);
            }
        }
        TransportPreparation prepared={};BsdfSample sample={};
        if (shading.s.is_hair) {
            // As in Cycles: R spends the glossy budget, orders crossing the fibre the transmission
            // budget; the continuation keeps the all-order mixture weight.
            bool reflected;
            sample.weight=strand_sample(shading.s.hair0,shading.s.hair1,shading.s.strand,shading.s.geometric,-direction,scatter.xyz,sample.direction,reflected,sample.pdf);
            sample.event=reflected ? REG_RAY_SPECULAR : REG_RAY_TRANSMISSION;
        } else {
            // Reflections retain the reconstructed smooth receiver when it faces
            // the incident ray. A back-facing smooth normal is outside the VNDF
            // domain; use the actual incident triangle face for that correction.
            let interface_face=dot(shading.s.geometric,-direction)>0. ? shading.s.geometric : geometric;
            prepared=transport_prepare(c,-direction,true,shading.s.geometric,interface_face);
            sample=transport_sample(c,-direction,scatter.xyz,prepared);
        }
        if (!all(isfinite(sample.direction)) || !all(isfinite(sample.weight)) || !isfinite(sample.pdf)) {
            if(path_boundary_transactions && solid) boundary_changes.finish(occupancy,false);
            path_boundary_error(REG_RADIANCE_NONFINITE,hit.object);break;
        }
        if(path_boundary_transactions && solid) boundary_changes.finish(occupancy,!shading.s.is_hair && sample.event==REG_RAY_TRANSMISSION
            && (transport_null_event(c,media.incident.ior,sample) || path_event_hemisphere(c,sample.event,sample.direction,geometric,shading.s.geometric)));
#ifdef VORK_PROPAGATION
        radiance+=clamp_sample(throughput*next_event<V,P>(shading,c,-direction,world,error,geometric,groups,transparent,media,subsurface_thickness,prepared,sample,sampler,optical_length,rng VORK_WORK_ARGUMENT),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        radiance+=clamp_sample(throughput*segment_next_event<V,P>(shading,c,-direction,world,error,geometric,media,groups,continuation_mask(bounce,diffuse,glossy,transmission),transparent,prepared,sampler,optical_length,rng),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
#else
        radiance+=clamp_sample(throughput*next_event<V,P>(shading,c,-direction,world,error,geometric,groups,transparent,media,subsurface_thickness,prepared,sample,sampler,optical_length,rng VORK_WORK_ARGUMENT),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        radiance+=clamp_sample(throughput*environment_next_event<V,P>(shading,c,-direction,world,error,geometric,media,groups,continuation_mask(bounce,diffuse,glossy,transmission),transparent,prepared,sampler,rng VORK_WORK_ARGUMENT),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        radiance+=clamp_sample(throughput*mesh_next_event<V,P>(shading,c,-direction,world,error,geometric,media,continuation_mask(bounce,diffuse,glossy,transmission),transparent,prepared,sampler,optical_length,rng VORK_WORK_ARGUMENT),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
        radiance+=clamp_sample(throughput*implicit_next_event<V,P>(shading,c,-direction,world,error,geometric,media,continuation_mask(bounce,diffuse,glossy,transmission),transparent,prepared,sampler,optical_length,rng VORK_WORK_ARGUMENT),select(bounce==0u,path_settings.clamp.x,path_settings.clamp.y),dropped_light_samples);
#endif

        if (!shading.s.is_hair && transport_null_event(c,media.incident.ior,sample)) {
            if (transparent++>=path_settings.limits.z) {break;}
            throughput*=sample.weight;
            if (!all(isfinite(throughput))) {path_boundary_error(REG_RADIANCE_NONFINITE,hit.object);break;}
            if (!any(throughput>float3(0.))) {break;}
            origin=path_origin(shading,world,error,direction);groups=incident_groups;
            pending_segment=max(0.,pending_segment-length(origin-world));preserve_segment=true;
            ray_event=sample.event;
            if(!path_boundary_transactions && solid) plain_path_cross(occupancy,boundary,entering);
            // A null crossing preserves the last scattering vertex and its MIS measure.
            continue;
        }

        if (bounce==path_settings.bounces.x) {break;}
        event=sample.event;ray_event=event;
        if (event==REG_RAY_DIFFUSE) {if (diffuse++>=path_settings.bounces.y) {break;}}
        else if (event==REG_RAY_SPECULAR) {if (glossy++>=path_settings.bounces.z || (diffuse>0u && (path_settings.limits.w&1u)==0u)) {break;}}
        else if (event==REG_RAY_TRANSMISSION) {if (transmission++>=path_settings.bounces.w || (diffuse>0u && (path_settings.limits.w&2u)==0u)) {break;}}
        else {break;}
        // Reflections follow the smooth receiver. Transmission must still
        // cross the actual medium boundary as well as the smooth hemisphere.
        if (!shading.s.is_hair && !path_event_hemisphere(c,event,sample.direction,geometric,shading.s.geometric)) {break;}
        if(!path_boundary_transactions && solid && event==REG_RAY_TRANSMISSION && !shading.s.is_hair) plain_path_cross(occupancy,boundary,entering);
        throughput*=sample.weight;
        if (!all(isfinite(throughput))) {path_boundary_error(REG_RADIANCE_NONFINITE,hit.object);break;}
        if (max(throughput.x,max(throughput.y,throughput.z))<=0.) {break;}
        if (bounce>=REG_PATH_ROULETTE_START) {
            // Radiance eta^-2 changes the measure, not path survival energy.
            // Compensate it for roulette so nested interfaces do not repeatedly
            // kill paths that recover that scale when they leave the medium.
            let survive=path_survival(throughput,N>0u ? occupancy.ior()/camera_ior : 1.);
            if (path_roulette(sampler)>survive) {break;}throughput/=survive;
        }
        // Hair has no environment or emissive-mesh next events: its continuation is their only strategy.
        previous_world=world;previous_pdf=shading.s.is_hair ? 0. : sample.pdf;
        direction=sample.direction;origin=path_origin(shading,world,error,direction);bounce++;
    }
    record_dropped_light_samples(shader_errors,dropped_light_samples);
    return radiance*frame.radiance.w;
}
// Scenes with programs compile their own entries with their Programs (SlangForward::path_module).
// Separable 1.5 px Blackman–Harris inverse CDF, prepared once on the host.
float path_filter_offset(float u) {
    let t=clamp(u,0.,1.)*float(REG_PATH_FILTER_INTERVALS);
    let i=min(uint(t),REG_PATH_FILTER_INTERVALS-1u);
    let a=asfloat(path_words[path_settings.limits.y+i]);
    let j=i+1u;
    let b=asfloat(path_words[path_settings.limits.y+j]);
    return lerp(a,b,t-float(i));
}
void path_pixel<let adaptive:bool,let spatial_subsurface:bool,V:IVariant, P:IProgram,let N:uint,let catchers:bool>(uint3 gid) {
    gid.y+=path_settings.boundaries.z;
    uint width,height;path_output.GetDimensions(width,height);if (gid.x>=width || gid.y>=height) {return;}
    set_material_texture_sample_count(PathShadows<V,P>.sample_count());
    let pixel=gid.y*width+gid.x;let index=uint(frame.temporal.x);
    // Moments reset on the exposure's first sample.
    PathMoment moment={};if(adaptive) {if(index>0u) {moment=path_moments[pixel];} else {path_moments[pixel].stopped=0u;}}
    let minimum=select(path_settings.sampling.w>0u,path_settings.sampling.w,max(16u,uint(sqrt(float(path_settings.sampling.x)))));
    let threshold=select(path_settings.adaptive.x>0.,path_settings.adaptive.x,.01);
    if (adaptive) {
        // Stopping is final, so the accumulator holds index times the stopped mean.
        // Both settled slots of a stopped pixel already read 1.
        if (moment.stopped!=0u) {
            path_output[gid.xy]=moment.mean;
            if(path_settings.environment.w!=0u) {
                path_guides[pixel*2u]*=float(index+1u)/float(index);
                path_guides[pixel*2u+1u].xyz*=float(index+1u)/float(index);
            }
            return;
        }
        var settled=false;
        if (index>=minimum) {
            let error=sqrt(max(moment.variance/float(max(index-1u,1u))/float(index),float4(0.)));
            settled=all(error<=threshold*max(abs(moment.mean),float4(.01)));
        }
        // Publish the test in this sample's parity slot and read neighbours' other slot,
        // which no invocation writes during this sample.
        path_moments[pixel].settled[index&1u]=uint(settled);
        // As in Cycles' convergence filter, stop only once the whole 3x3 neighbourhood
        // passed on the previous sample: a thin strand can miss a pixel's first samples.
        var done=settled;
        for (var dy=-1;dy<=1 && done;dy++) {
            for (var dx=-1;dx<=1 && done;dx++) {
                let n=uint2(clamp(int2(gid.xy)+int2(dx,dy),int2(0),int2(int(width),int(height))-1));
                done=path_moments[n.y*width+n.x].settled[(index&1u)^1u]!=0u;
            }
        }
        if (done) {
            path_moments[pixel].stopped=1u;path_output[gid.xy]=moment.mean;
            if(path_settings.environment.w!=0u) {
                path_guides[pixel*2u]*=float(index+1u)/float(index);
                path_guides[pixel*2u+1u].xyz*=float(index+1u)/float(index);
            }
            return;
        }
    }
    let animated=select((path_settings.sampling.z&4u)!=0u,pixel_hash(asuint(frame.scene.x)),0u);
    var rng=screen_ray_hash(gid.xy,index,path_settings.sampling.y^animated);
    let sampler=path_sampler(gid.xy,index,uint(frame.temporal.y),path_settings.sampling.y^animated);
    // This pixel's own pixel and lens point. The shared projection carries one
    // pixel jitter and lens offset for all pixels (raster's); move this ray from
    // that camera to the drawn one: clip shifts are NDC offsets, the lens offset
    // translates the eye along the camera axes. lens.x is the sampled aperture radius.
    let camera_sample=path_camera_sample(sampler);
    let shared=frame.camera_sample;
    var lens=float2(0.);var focus=float2(0.);
    if (frame.lens.x>0.) {
        lens=aperture_point(polar_disk(camera_sample.zw),frame.aperture)*frame.lens.x;
        focus=(lens-shared.zw)*frame.projection.yx/frame.lens.y;
    }
    let ndc=(float2(gid.xy)+.5+float2(path_filter_offset(camera_sample.x),path_filter_offset(camera_sample.y)))/float2(width,height)*float2(2.,-2.)+float2(-1.,1.)+shared.xy-focus;
    let eye=float3(frame.view[0].x,frame.view[1].x,frame.view[2].x)*(lens.x-shared.z)+float3(frame.view[0].y,frame.view[1].y,frame.view[2].y)*(lens.y-shared.w);
    // Environment programs observe the actual aperture/view-plane origin.
    // Near clipping bounds scene visibility, not the observer's position.
    float3 camera_origin;float3 camera_direction;
    camera_ray_line(ndc,frame,camera_origin,camera_direction);camera_origin+=eye;
    var direction=camera_direction;
    let forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
    let near_distance=frame.params.x/dot(direction,forward);
    var camera_min=near_distance;var camera_max=frame.params.y/dot(direction,forward);
    var origin=camera_origin+direction*near_distance;float alpha;
#ifdef VORK_LIGHT_PATHS
    PathCameraContext ctx;ctx.film_size=float2(width,height);ctx.uv=(float2(gid.xy)+.5+float2(path_filter_offset(camera_sample.x),path_filter_offset(camera_sample.y)))/ctx.film_size;ctx.lens_sample=camera_sample.zw;ctx.time=frame.scene.x;
    ctx.focus_distance=asfloat(frame.camera_program.w);
    ctx.right=float3(frame.view[0].x,frame.view[1].x,frame.view[2].x);
    ctx.up=float3(frame.view[0].y,frame.view[1].y,frame.view[2].y);
    ctx.forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
    ctx.origin=frame.eye.xyz-ctx.right*shared.z-ctx.up*shared.w;
    let ray=camera_dispatch(frame.camera_program.x,frame.camera_program.y,ctx);
    if(!all(isfinite(ray.origin)) || !all(isfinite(ray.direction)) || dot(ray.direction,ray.direction)<=0.
        || !all(isfinite(ray.weight)) || !isfinite(ray.wavelength_scale) || ray.wavelength_scale<=0.
        || !isfinite(ray.t_min) || !isfinite(ray.t_max) || ray.t_min<0. || ray.t_max<ray.t_min) {
        path_boundary_error(REG_CAMERA_RAY_INVALID,frame.camera_program.x);return;
    }
    direction=normalize(ray.direction);camera_origin=ray.origin;
    camera_min=ray.t_min;camera_max=ray.t_max;
    origin=camera_origin+direction*camera_min;
    path_sensor_context.uv=ctx.uv;path_sensor_context.time=ctx.time;
    path_sensor_context.wavelength=((frame.camera_program.z&1u)!=0u ? REG_SPECTRAL_MIN_NM+REG_SPECTRAL_SPAN_NM*path_wavelength_sample(sampler) : 0.);path_wavelength_scale=ray.wavelength_scale;
    vork_path_wavelength=path_sensor_context.wavelength*path_wavelength_scale;
#endif
#ifdef VORK_PROPAGATION
    origin=camera_origin;
#endif
#ifdef VORK_MEDIUM_WORK_LIMIT
    PathMediumWork medium_work={VORK_MEDIUM_WORK_LIMIT,64u,false};
#endif
#ifndef VORK_PROPAGATION
    if(path_fields<V>().y>0u || (frame.camera_program.z&2u)!=0u) {
        var control_origin=origin;var control_direction=direction;
        let primary=trace<V,P>(control_origin,control_direction,max(0.,camera_max-camera_min),REG_RAY_CAMERA VORK_WORK_ARGUMENT);
        float2 depth=float2(1.);float4 normal=float4(0.);uint id=0u;
        if(primary.t>=0.) {
            let h=dielectric_surface<V,P,PathGeometry>(primary,control_direction,float2(2.)/float2(width,height),Lobe(0u,0.));
            let clip=mul(float4(h.v.world,1.),frame.vp);let distance=length(h.v.world-camera_origin);depth=float2((frame.camera_program.z&2u)!=0u ? distance/(distance+1.) : clip.z/clip.w);
            normal=float4(normalize(h.v.NORMAL),0.);id=uint(objects[primary.object].opacity.y);
        }
        field_depth[gid.xy]=depth;field_normals[gid.xy]=normal;field_ids[gid.xy]=id;
    }
#endif
    let occupancy_initial=camera_occupancy<V,P,PathGeometry,N>(path_scene,origin,float3(1.,0.,0.),path_settings.boundaries.xw,float2(2.)/float2(path_image_size()),path_fields<V>().x,path_fields<V>().y VORK_WORK_ARGUMENT);
    var occupancy=occupancy_initial;
#ifdef VORK_LIGHT_PATHS
    for(var k=0u;k<occupancy.count;k++) {
        var b=occupancy.at(k);b.color=path_spectrum(b.color);b.ior=spectral_ior(b.ior,vork_path_wavelength,b.dispersion_scale,b.dispersion_abbe);occupancy.set(k,b);
    }
#endif
    float4 aovs[VORK_AOV_SLOTS];
    float4 guide_albedo;float4 guide_normal;
    catcher_enabled=catchers;
    uint camera_flags;let initial_rng=rng;
    float4 primary_position;float4 primary_normal;
    var value=float4(integrate<spatial_subsurface,V,P,N>(origin,direction,camera_origin,camera_min,camera_max,occupancy,sampler,rng,alpha,aovs,guide_albedo,guide_normal,camera_flags,primary_position,primary_normal VORK_WORK_ARGUMENT),alpha);
#ifdef VORK_PROPAGATION
    let primary_clip=mul(primary_position,frame.vp);
    let primary_distance=length(primary_position.xyz-camera_origin);
    field_depth[gid.xy]=primary_position.w>0. ? float2((frame.camera_program.z&2u)!=0u ? primary_distance/(primary_distance+1.) : primary_clip.z/primary_clip.w) : float2(1.);
    field_normals[gid.xy]=float4(primary_normal.xyz,0.);field_ids[gid.xy]=uint(primary_normal.w);
#endif
#ifdef VORK_LIGHT_PATHS
    value.rgb*=ray.weight;
    if((frame.camera_program.z&1u)!=0u) {guide_albedo.w=1.;guide_normal.w=-1.;}
#endif
    if(catchers) {
        var lit=float4(0.);var baseline=float4(0.,0.,0.,camera_flags==0u ? 1. : 0.);var background=float4(0.,0.,0.,1.);
        if((camera_flags&REG_OBJECT_CATCHER)!=0u) {
            lit=float4(value.rgb,1.);
            catcher_without_cg=true;rng=initial_rng;
            // Rebuild camera containment after removing CG, including implicit solids.
            var reference_occupancy=camera_occupancy<V,P,PathGeometry,N>(path_scene,origin,float3(1.,0.,0.),path_settings.boundaries.xw,float2(2.)/float2(path_image_size()),path_fields<V>().x,path_fields<V>().y VORK_WORK_ARGUMENT);
#ifdef VORK_LIGHT_PATHS
            for(var k=0u;k<reference_occupancy.count;k++) {
                var boundary=reference_occupancy.at(k);boundary.color=path_spectrum(boundary.color);boundary.ior=spectral_ior(boundary.ior,vork_path_wavelength,boundary.dispersion_scale,boundary.dispersion_abbe);reference_occupancy.set(k,boundary);
            }
#endif
            float reference_alpha;float4 reference_aovs[VORK_AOV_SLOTS];uint reference_flags;float4 reference_albedo;float4 reference_normal;
            float4 reference_position;float4 reference_primary_normal;
            baseline.rgb=integrate<spatial_subsurface,V,P,N>(origin,direction,camera_origin,camera_min,camera_max,reference_occupancy,sampler,rng,reference_alpha,reference_aovs,reference_albedo,reference_normal,reference_flags,reference_position,reference_primary_normal VORK_WORK_ARGUMENT);
#ifdef VORK_LIGHT_PATHS
            baseline.rgb*=ray.weight;
#endif
            catcher_without_cg=false;
            value=float4(0.);baseline.w=1.;guide_albedo=float4(0.);guide_normal=float4(0.);
            background.rgb=frame.environment.w==0. ? frame.background.rgb : path_environment(direction,REG_RAY_CAMERA,0u,camera_origin);
        }
        if(index>0u) {lit+=path_catcher[pixel*3u];baseline+=path_catcher[pixel*3u+1u];background+=path_catcher[pixel*3u+2u];}
        path_catcher[pixel*3u]=lit;path_catcher[pixel*3u+1u]=baseline;path_catcher[pixel*3u+2u]=background;
        let ratio=select(baseline.rgb!=float3(0.),lit.rgb/select(baseline.rgb!=float3(0.),baseline.rgb,float3(1.)),float3(1.));
        let coverage=lit.w/max(baseline.w,1.);
        let result=lerp(float3(1.),ratio,coverage);
        for(var c=0u;c<3u;c++) {path_pixels[(path_settings.matte.x+c)*width*height+pixel]=asuint(result[c]);}
    }
    if(path_settings.environment.w!=0u) {
        if(index>0u) {
            guide_albedo+=path_guides[pixel*2u];
            let previous=path_guides[pixel*2u+1u];guide_normal.xyz+=previous.xyz;
            if(previous.w!=guide_normal.w) {guide_normal.w=-1.;}
        }
        path_guides[pixel*2u]=guide_albedo;path_guides[pixel*2u+1u]=guide_normal;
    }
    if (VORK_AOV_CHANNELS!=0u) for(var a=0u;a<path_settings.environment.z;a++) {path_aovs[pixel*path_settings.environment.z+a]=aovs[a];}
    if (!all(isfinite(value))) {path_boundary_error(REG_RADIANCE_NONFINITE,pixel);}
    let delta=value-moment.mean;moment.mean+=delta/float(index+1u);moment.variance+=delta*(value-moment.mean);
    // Field writes leave the settled slots, which neighbours read, untouched.
    if (adaptive) {path_moments[pixel].mean=moment.mean;path_moments[pixel].variance=moment.variance;}
    path_output[gid.xy]=value;
}

[shader("compute")]
[numthreads(8,8,1)]
void path_main(uint3 gid:SV_DispatchThreadID) { path_pixel<true,true,Unspecialized,NoPrograms,0u,false>(gid); }
[shader("compute")]
[numthreads(8,8,1)]
void path_fixed(uint3 gid:SV_DispatchThreadID) { path_pixel<false,true,Unspecialized,NoPrograms,0u,false>(gid); }
