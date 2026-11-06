// Interface-by-interface raster continuation. Geometry, occupancy and tint are
// shared with path transport; local receiver lighting uses the forward evaluator.
import vork_constants;
import frame;
import material;
import surface;
import stream;
import variant;
import forward;
import rays;
import vork_ray_alpha;
import hit;
import emitter_mesh;
import ray_coverage;
import hit_lighting;
import openpbr_core;
import dielectric_scene;
import path_boundary;
import camera_config;
import bsdf;
import path_sampler;
#ifdef VORK_BOUNDED_MEDIUM
import vork_medium_types;
import vork_medium_bounds;
import refraction_medium;
#endif
#ifndef VORK_REFRACTION_BOUNDARIES
#define VORK_REFRACTION_BOUNDARIES 0
#endif
// Sampled secondary lighting uses physical source connections. Explicit LTC
// keeps main's filtered map approximation at the actual receiver, never the
// camera slot's coverage offsets (which belong to a different surface).
struct RefractionShadows<V:IVariant,P:IProgram> : IShadows {
    static const bool probe_rays=true;
    static bool probe_visible(float3 world,float3 normal,float3 error,float3 probe) {return RasterShadows<V>.probe_visible(world,normal,error,probe);}
    static const bool subsurface=V.subsurface;
    static uint integration_count(uint index,out float3 normalization) {let count=index==0xffffffffu ? sample_count() : 1u;normalization=float3(float(count));return count;}
    static float3 integration_sample(uint tap,inout uint index,inout float3 world) {if(index==0xffffffffu) {index=tap;}return float3(1.);}
    static uint sample_count() {return RasterShadows<V>.sample_count();}
    static float thickness(uint object,float3 world,float3 direction) {return RasterShadows<V>.thickness(object,world,direction);}
    static float3 visibility_direction(float3 world,float3 normal,LightValues record,uint light,float3 direction,float distance,float3 error,uint index,uint count,bool fibre=false) {return hit_shadow_direction<P,ReceiverGeometry>(ray_scene,ray_alpha,light,world,normal,error,direction,distance,index,count);}
}
DielectricHit refraction_boundary<V:IVariant,P:IProgram>(float3 origin,float3 direction,float2 pixel,uint event) {
    RayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=0.;ray.TMax=REG_FP32_MAX;
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(ray_scene,RAY_FLAG_FORCE_NON_OPAQUE,event,ray);
    // Two triangles sharing an edge can report the same t. Commit order follows
    // the acceleration structure's traversal, which differs between builds, so a
    // tie keeps the lower (object, primitive): the same surface every frame.
    // Every triangle is a candidate here, so the choice never depends on order.
    DielectricHit h={};h.t=-1.;
    while(query.Proceed()) {
        if(query.CandidateType()!=CANDIDATE_NON_OPAQUE_TRIANGLE) {continue;}
        let candidate=RayIntersection(query.CandidateInstanceIndex(),query.CandidateInstanceID(),query.CandidatePrimitiveIndex(),query.CandidateTriangleBarycentrics());
        if(!candidate_accepts<P,ReceiverGeometry>(ray_alpha,candidate,query.CandidateWorldToObject3x4(),direction,pixel)) {continue;}
        let t=query.CandidateTriangleRayT();
        if(h.t>=0. && (t>h.t || (t==h.t && (candidate.instance_custom_data>h.object || (candidate.instance_custom_data==h.object && candidate.primitive_index>=h.primitive))))) {continue;}
        query.CommitNonOpaqueTriangleHit();
        h.t=t;h.instance=candidate.instance_index;h.object=candidate.instance_custom_data;h.primitive=candidate.primitive_index;
        h.barycentrics=candidate.barycentrics;h.world_to_object=query.CandidateWorldToObject3x4();
    }
    return merge_field_hits<P,ReceiverGeometry>(h,frame.field_geometry.xy,origin,direction,ray.TMax,event);
}
float3 refraction_environment(uint object,float3 direction,float roughness,uint event) {
    if(!light_receives(frame.environment_links.x,frame.environment_links.y,objects[object].metadata.x)
        || (frame.environment_links.x&event)==0u) {return float3(0.);}
    var value=frame.radiance.xyz;
    if(frame.environment.x>0.) {
        value+=frame.environment.x*frame.environment.y*frame.environment_color.rgb*env_specular.SampleLevel(env_sampler,environment_uv(direction,frame.environment_rotation),roughness*frame.environment.z).rgb;
    }
    return value*frame.radiance.w;
}
float3 refraction_transport<V:IVariant,P:IProgram,let N:uint>(Varying primary,Surface surface,Coordinates coordinates,float3 camera_origin,PathOccupancy<N> camera_state,float3 facet,float2 pixel,uint2 seed,uint sample_index) {
#ifdef VORK_BOUNDED_MEDIUM
    refraction_medium_sample(sample_index,RefractionShadows<V,P>.sample_count());
#endif
    let tables=camera_config.boundaries.xy;
    var direction=-surface.view;
    var occupancy=camera_state;
    var identity=ray_alpha[camera_config.boundaries.z+primary.object];
    if(surface.closure.transmission_weight>0. && !surface.closure.geometry_thin_walled) {validate_boundary<V,P>(primary,surface.front_facing,coordinates,surface);}
    var boundary=path_boundary(identity,surface.closure,material_record(primary.object));
    var v=primary;var s=surface;var geometric=facet;
    reconcile_occupancy<ReceiverGeometry,N>(occupancy,primary.world,identity,tables);
    var throughput=occupancy.attenuation(length(primary.world-camera_origin));var radiance=float3(0.);
    let camera_ior=occupancy.ior();
    var events=0u;
    var sampler=path_sampler(seed,sample_index,frame.integrator.z,camera_config.seed);
    for(;;) {
        let c=s.closure;
        let solid=!c.geometry_thin_walled && c.transmission_weight>0.;
        let incident=occupancy.current();
        let transmitted=solid ? occupancy.transmitted(boundary,s.front_facing) : incident;
        var relative=s;
        relative.closure=dielectric_interface(c,incident.ior,solid ? transmitted.ior : c.specular_ior);
        // The closure is already relative to this incident medium. Its normal
        // faces the incident side, so the forward evaluator must not invert it.
        relative.front_facing=true;
        let site=hit_site(v,coordinates,geometric,seed,sample_index,true);
        float3 scattering;float3 indirect_scattering;
        // Continuation replaces this glossy baseline at every transmitting
        // interface. Omit the estimate instead of computing and subtracting it,
        // including builds without participating media.
        let glossy_ambient=c.transmission_weight<=0. || s.is_hair || (P.lit && P.lit_material(v));
        var sampled_sources=false;
#ifdef VORK_BOUNDED_MEDIUM
        // Bound surface-light fog connections as well as interval sources.
        sampled_sources=true;
#endif
        let lighting=AnalyticRadiance(REG_RAY_DIFFUSE|REG_RAY_SPECULAR,true,false,false,glossy_ambient,sampled_sources);
        let local=analytic_radiance_at<V,RefractionShadows<V,P>>(v,relative,site,lighting,scattering,indirect_scattering);
        if(c.transmission_weight<=0. || s.is_hair || (P.lit && P.lit_material(v))) {return radiance+throughput*local.color;}
        // Replace the environment specular estimate with the traced BSDF
        // continuation. Keep finite-light integration and diffuse ambient.
        radiance+=throughput*(local.color-reflection_bsdf_environment(local.reflection));
        let prepared=transport_prepare(relative.closure,-direction,true,relative.geometric,geometric);
        let sample=transport_sample(relative.closure,-direction,path_scatter(sampler).xyz,prepared);
        if(!all(isfinite(sample.direction)) || !all(isfinite(sample.weight)) || !isfinite(sample.pdf)) {
            InterlockedMax(shader_errors[0],REG_RADIANCE_NONFINITE);return radiance;
        }
        if(sample.event!=REG_RAY_SPECULAR && sample.event!=REG_RAY_TRANSMISSION) {return radiance;}
        if((sample.event==REG_RAY_TRANSMISSION)!=(dot(geometric,sample.direction)<0.)) {return radiance;}
        let outgoing=sample.direction;
        throughput*=sample.weight;
        // The camera's reflection provider applies at the first interface.
        // Rear reflections remain part of dielectric transmission transport.
        if(events==0u && sample.event==REG_RAY_SPECULAR && frame.projection.z<REG_REFLECTIONS_RAY) {
            return radiance+(frame.projection.z==REG_REFLECTIONS_OFF ? float3(0.)
                : throughput*refraction_environment(v.object,outgoing,0.,REG_RAY_SPECULAR));
        }
        if(solid && sample.event==REG_RAY_TRANSMISSION) {path_cross(occupancy,boundary,s.front_facing);}
        if(!all(isfinite(throughput)) || !any(throughput>float3(0.))) {return radiance;}
        // Match path transport's eta-compensated roulette. There is no maximum
        // boundary count: surviving paths carry the discarded paths' energy.
        if(events++>=REG_PATH_ROULETTE_START) {
            let survive=path_survival(throughput,occupancy.ior()/camera_ior);
            if(path_roulette(sampler)>survive) {return radiance;}
            throughput/=survive;
        }
        var origin=surface_ray_origin(v.world,surface_position_error(v.object,v.world),geometric,outgoing);
        direction=outgoing;
        for(;;) {
            let hit=refraction_boundary<V,P>(origin,direction,pixel,sample.event);
#ifdef VORK_BOUNDED_MEDIUM
            // Escape still crosses participating media, including an infinite
            // world tail. Dielectric absorption never invents an infinite chord.
            let interior=medium_transport_segment(origin,direction,hit.t<0. ? medium_infinity() : medium_receiver(hit.t),path_subsurface_entry(sampler),v.object);
            if(!all(isfinite(interior.source)) || !all(isfinite(interior.transmission))) {
                InterlockedMax(shader_errors[0],REG_PATH_MEDIUM_DIVERGENT);return radiance;
            }
            radiance+=throughput*interior.source;throughput*=interior.transmission;
#endif
            if(hit.t<0.) {return radiance+throughput*refraction_environment(v.object,direction,0.,sample.event);}
            sampler.vertex++;let pick=path_scatter(sampler).w;
            var h=dielectric_surface<V,P,ReceiverGeometry>(hit,direction,pixel,Lobe(select(P.lobes,LOBE_PICK,0u),pick));
            reconcile_occupancy<ReceiverGeometry,N>(occupancy,h.v.world,instance_solid<ReceiverGeometry>(hit.instance,tables),tables);
            throughput*=occupancy.attenuation(hit.t);
            let u=path_opacity(sampler);
            let record=ray_alpha[hit.instance*REG_RAY_SLOT_WORDS+1u];
            let opacity=h.s.closure.geometry_opacity*objects[hit.object].opacity.x;
            if(u>=opacity) {origin=surface_ray_origin(h.v.world,surface_position_error(hit.object,h.v.world),h.face,direction);continue;}
            let solid_hit=!h.s.closure.geometry_thin_walled && h.s.closure.transmission_weight>0.;
            boundary=solid_hit ? boundary_descriptor<V,P,ReceiverGeometry>(hit,direction,h,tables,pixel) : path_boundary(0xffffffffu,h.s.closure);
            if(P.lit && P.lit_material(h.v)) {radiance+=throughput*emitter_mesh_radiance<V,P>(h)*frame.radiance.w;h.s=lit_surface<V,RefractionShadows<V,P>,P>(h.v,h.entering,h.k,h.s,hit_site(h.v,h.k,h.face,seed,sample_index,true));}
            if(P.lobes) {throughput*=h.s.weight;}
            v=h.v;s=h.s;coordinates=h.k;geometric=h.face;break;
        }
    }
}
