#ifndef VORK_MEDIUM_NO_ANALYTIC_LIGHTS
#define VORK_MEDIUM_NO_ANALYTIC_LIGHTS 0
#endif
import vork_prelude;
import vork_constants;
import vork_medium_point;
import vork_light_falloff;
import vork_volume_settings;
import vork_sampling;
import medium_environment;
internal extern Frame medium_frame();
internal extern Light medium_light(uint index);
internal extern uint medium_light_list(float3 position,out uint base);
internal extern uint medium_light_index(uint base,uint next);
// Visibility is independent of the receiving ray. Keep light identity until
// the reader evaluates the complete HG lobe, including its forward peak.
public float medium_analytic_weight() {
#ifdef VORK_MEDIUM_SAMPLED_ANALYTIC_LIGHT
    return float(frame_light_count(medium_frame()));
#else
    return 1.;
#endif
}
public float medium_phase_weight(float3 axis,float3 ray,float g) {
    return medium_phase(g,dot(axis,ray));
}
internal extern float3 medium_cached_visibility(float3 position,uint light);
internal extern float3 medium_direct_visibility(float3 position,uint light);
public float3 medium_limit_light(float3 value) {
    if(VOLUME_LIGHT_CLAMP>0.) {
        value*=min(1.,VOLUME_LIGHT_CLAMP/max(max(value.x,max(value.y,value.z)),REG_POSITIVE_EPSILON));
    }
    return value;
}
public float3 medium_shared_illumination(float3 position,float3 ray,float g) {
    let f=medium_frame();
#ifdef VORK_MEDIUM_ENVIRONMENT
    var incoming=medium_environment_illumination(position,ray,g);
#else
    var incoming=(f.medium.w&1u)!=0u ? f.radiance.rgb+f.environment_mean.rgb*f.environment_color.rgb*f.environment.x : float3(0.);
#endif
    uint base=0u;uint count=0u;
#if !VORK_MEDIUM_NO_ANALYTIC_LIGHTS
    count=medium_light_list(position,base);
#endif
    for(uint next=0u;next<count;next++) {
        let i=medium_light_index(base,next);let light=medium_light(i);
        if((light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]&REG_RAY_VOLUME)==0u || all(light.radiance.rgb==float3(0.))) {continue;}
        let falloff=light_falloff(position,light.position,light.direction,light.radiance.w,light.projection.y,light.projection.z>0.);
        if(falloff.punctual<=0.) {continue;}
        let radiance=light.radiance.rgb*(falloff.punctual*medium_phase_weight(falloff.axis,ray,g));
        var value=medium_limit_light(radiance*medium_cached_visibility(position,i));
        if(!all(isfinite(value))) {
            incoming+=medium_analytic_weight()*medium_limit_light(radiance*medium_direct_visibility(position,i));
            continue;
        }
        // E[C + I/p*(D-C) | C,D] = D. The cached field is a control variate,
        // not the transport integrand. Complete support includes clamped exterior
        // lookups and blockers smaller than a cell. Clamp each integrand BEFORE
        // taking the residual; never clamp the signed correction itself.
        let key=asuint(position.xy)^uint2(pixel_hash(asuint(position.z)),pixel_hash(i));
        let index=uint(f.temporal.x);let strata=REG_MEDIUM_RESIDUAL_STRATA;
        // Independently choose one member of each pair. Hashing the pair avoids
        // correlating this selector with the emitter's low-discrepancy coordinate.
        let choice=screen_ray_hash(key,index/strata,REG_SAMPLING_STREAM_VOLUME_RESIDUAL)%strata;
        if(index%strata==choice) {
            let direct=medium_limit_light(radiance*medium_direct_visibility(position,i));
            value+=(direct-value)*float(strata);
        }
        incoming+=medium_analytic_weight()*value;
    }
    return incoming;
}
