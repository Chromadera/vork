#ifndef VORK_MEDIUM_WORLD_ONLY
#define VORK_MEDIUM_WORLD_ONLY 0
#endif
#ifndef VORK_MEDIUM_BOUNDARIES
#define VORK_MEDIUM_BOUNDARIES 0
#endif
#ifndef VORK_HOMOGENEOUS_MEDIA
#define VORK_HOMOGENEOUS_MEDIA 0
#endif
#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
#ifndef VORK_MEDIUM_STEPS
#define VORK_MEDIUM_STEPS REG_MEDIUM_STEPS
#endif
import vork_constants;
import vork_volume_settings;
import vork_prelude;
import vork_medium;
import vork_medium_point;
internal extern Frame medium_frame();
import frame;
// The consuming pass exports the reflected point dispatch.
// Its generated evaluator reads context.input_base/inverse here.
internal extern MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext context);
internal extern bool volume_homogeneous(uint id);
internal extern MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext context);
__exported import vork_medium_domain;
public MediumTransport volume_finite<let Steps:int,let Lit:bool,let Shared:bool=false>(float3 o,float3 d,MediumEndpoint endpoint,float sample_fraction,uint inside_object,bool camera_ray,float scale=1.) {
    let base=medium_frame().medium.x;let count=volume_domain_count();let descriptors=base+medium_word(base);
    var source=float3(0.);var transmission=float3(1.);var optical_depth=float3(0.);var start=0.;
    let infinite=endpoint.kind==MediumEndpointKind.Infinity;let distance=infinite ? REG_FP32_MAX : endpoint.distance;
    // Each segment ends at a true mesh intersection, so thin and disjoint volumes
    // receive the same point-step budget as large ones. No fixed crossing limit.
    for (;;) {
        if (start>=distance) {break;}
        // A boundary at (or rounding to) `start` would repeat this interval
        // forever on the GPU; every interval advances by a distinguishable step.
        let after=min(volume_interval_successor(start),distance);
        let interval=volume_boundary_interval(o,d,after,distance,inside_object,camera_ray);
        let end=interval.end;
        if(infinite && end==REG_FP32_MAX) {
            let tail=medium_world_tail<Lit>(o+d*start,d,scale,sample_fraction);
            source+=medium_received_source(transmission,tail.source);transmission*=tail.transmission;break;
        }
        let dt=(end-start)/float(Steps);
        let quadrature_fraction=Lit ? medium_quadrature_fraction(sample_fraction,uint(Steps)) : sample_fraction;
        // A sum of homogeneous media is homogeneous even when their colours,
        // emission and phase functions differ. Extinction and emission therefore
        // integrate exactly once per boundary interval. Only incident lighting
        // needs spatial quadrature. Streaming each medium's weighted lighting
        // avoids sample arrays whose live ranges cross light/shadow evaluation.
#if VORK_HOMOGENEOUS_MEDIA
        MediumExtinction sum={};
        for(var i=0u;i<=count;i++) {
            let instance=volume_interval_instance(i,interval,o,d,(start+end)*.5,distance,inside_object,camera_ray);
            let id=instance.program;let context=instance.context;
            if(id==0xffffffffu) {continue;}
            let m=volume_coefficients(id,o+d*start,context);
            medium_extinction_add(sum,m);
        }
        let sigma=sum;
        let span=end-start;
        let attenuation=exp(-min(medium_optical_depth(sigma,dt),float3(REG_MEDIUM_EXP_LIMIT)));
        let interval_transmission=exp(-min(medium_optical_depth(sigma,span),float3(REG_MEDIUM_EXP_LIMIT)));
        var interval_source=float3(0.);
        for(var i=0u;i<=count;i++) {
            let instance=volume_interval_instance(i,interval,o,d,(start+end)*.5,distance,inside_object,camera_ray);
            let id=instance.program;let context=instance.context;
            if(id==0xffffffffu) {continue;}
            let m=volume_coefficients(id,o+d*start,context);
            // Extinction above needs no light connection. An explicit
            // demand test also lets native compilation discard all lighting
            // for non-scattering authored programs (0*light cannot fold under
            // precise IEEE arithmetic when light may be non-finite).
            if(!Lit) {continue;}
            interval_source+=medium_integrated_coefficient(m.emission,sigma,span,interval_transmission);
            if(!any(m.scattering>float3(0.))) {continue;}
            var lighting=float3(0.);
            // Reverse Horner evaluation has the same quadrature weights with
            // one less RGB accumulator live across the lighting evaluator.
            for(var step=Steps;step>0u;step--) {
                var incident=float3(0.);
                if(medium_lighting_cell(uint(step-1),sample_fraction,uint(Steps))) {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
                    float offset;
                    let weight=medium_extinction_sample(sigma,dt,quadrature_fraction,offset);
                    let p=o+d*(start+float(step-1u)*dt+offset);
                    incident=weight*(Shared ? medium_directional_illumination(p,d,m.anisotropy) : medium_illumination(p,d,m.anisotropy));
#else
                    // Each channel integrates lighting against its own extinction
                    // measure. Equal extinctions share one point query, including
                    // geometry visibility and the world-field residual.
                    [loop] for(uint channel=0u;channel<3u;channel++) {
                        if(m.scattering[channel]<=0.) {continue;}
                        var duplicate=false;
                        for(uint earlier=0u;earlier<channel;earlier++) {
                            duplicate=duplicate || (medium_extinction_equal(sigma,earlier,channel) && m.scattering[earlier]>0.);
                        }
                        if(duplicate) {continue;}
                        let offset=medium_extinction_distance(sigma,channel,dt,quadrature_fraction);
                        let p=o+d*(start+float(step-1u)*dt+offset);
                        // This is a point draw from a nonuniform measure; a
                        // uniform longitudinal footprint would change that measure.
                        let light=Shared ? medium_directional_illumination(p,d,m.anisotropy) : medium_illumination(p,d,m.anisotropy);
                        for(uint matched=channel;matched<3u;matched++) {
                            if(medium_extinction_equal(sigma,matched,channel)) {incident[matched]=light[matched];}
                        }
                    }
#endif
                }
                lighting=incident*medium_lighting_weight(uint(Steps))+attenuation*lighting;
            }
            interval_source+=medium_integrated_coefficient(m.scattering,sigma,dt,attenuation)*lighting;
        }
        source+=medium_received_source(transmission,interval_source);transmission*=interval_transmission;optical_depth+=medium_optical_depth(sigma,span);
#elif VORK_MEDIUM_WORLD_ONLY
        // With no bounded domains there is nothing to overlap. Consume each
        // cell immediately instead of keeping its coefficients/source live
        // across all lighting queries. Preserve the same ordered quadrature.
        let instance=volume_interval_instance(0u,interval,o,d,(start+end)*.5,distance,inside_object,camera_ray);
        if(instance.program!=0xffffffffu) {
            let homogeneous=volume_homogeneous(instance.program);var constant=MediumSample();
            if(homogeneous) {constant=volume_coefficients(instance.program,o+d*start,instance.context);}
            var shadow_sigma=float3(0.);
            for(var step=0u;step<Steps;step++) {
                var m=constant;
                if(!homogeneous) {m=volume_interval_coefficients(instance.program,o+d*(start+float(step)*dt),d,dt,quadrature_fraction,instance.context);}
                MediumExtinction sigma={};medium_extinction_add(sigma,m);
                if(Lit) {
                    let attenuation=exp(-min(medium_optical_depth(sigma,dt),float3(REG_MEDIUM_EXP_LIMIT)));
                    var incident=medium_integrated_coefficient(m.emission,sigma,dt,attenuation);
                    if(any(m.scattering>float3(0.)) && medium_lighting_cell(uint(step),sample_fraction,uint(Steps))) {
                        let p=o+d*(start+(float(step)+quadrature_fraction)*dt);
                        incident+=medium_integrated_coefficient(m.scattering,sigma,dt,attenuation)
                            *(Shared ? medium_directional_illumination(p,d,m.anisotropy) : medium_illumination(p,d,m.anisotropy,d*dt,quadrature_fraction))*medium_lighting_weight(uint(Steps));
                    }
                    source+=medium_received_source(transmission,incident);transmission*=attenuation;optical_depth+=medium_optical_depth(sigma,dt);
                } else {shadow_sigma+=medium_optical_depth(sigma,dt);}
            }
            if(!Lit) {let tau=shadow_sigma;transmission*=exp(-min(tau,float3(REG_MEDIUM_EXP_LIMIT)));optical_depth+=tau;}
        }
#else
        // Optical depth is linear: a source-free connection needs one
        // accumulator, rather than retaining every quadrature cell.
        MediumExtinction aggregates[Lit ? Steps : 1];
        float3 incident[Lit ? Steps : 1];
        for(var step=0u;step<(Lit ? Steps : 1);step++) {
            aggregates[step]=MediumExtinction(float3(0.),float3(0.));incident[step]=float3(0.);
        }
        for(var i=0u;i<=count;i++) {
            let instance=volume_interval_instance(i,interval,o,d,(start+end)*.5,distance,inside_object,camera_ray);
            let id=instance.program;let context=instance.context;
            if(id==0xffffffffu) {continue;}
            let homogeneous=volume_homogeneous(id);
            var constant=MediumSample();
            if(homogeneous) {constant=volume_coefficients(id,o+d*start,context);}
            for(var step=0u;step<Steps;step++) {
                let p=o+d*(start+(float(step)+quadrature_fraction)*dt);
                var m=constant;
                if(!homogeneous) {m=volume_interval_coefficients(id,o+d*(start+float(step)*dt),d,dt,quadrature_fraction,context);}
                // Add overlapping point coefficients before integrating once.
                // No exp/log transport round trip, or division by an integral.
                let slot=Lit ? step : 0u;
                medium_extinction_add(aggregates[slot],m);
            }
        }
        if(Lit) {
            // Total extinction must be known before weighting each overlapping
            // source. Stream a second coefficient pass instead of retaining a
            // per-domain lighting array or overflowing source-density products.
            for(var i=0u;i<=count;i++) {
                let instance=volume_interval_instance(i,interval,o,d,(start+end)*.5,distance,inside_object,camera_ray);
                let id=instance.program;let context=instance.context;if(id==0xffffffffu) {continue;}
                let homogeneous=volume_homogeneous(id);var constant=MediumSample();
                if(homogeneous) {constant=volume_coefficients(id,o+d*start,context);}
                for(var step=0u;step<Steps;step++) {
                    let p=o+d*(start+(float(step)+quadrature_fraction)*dt);
                    var m=constant;
                    if(!homogeneous) {m=volume_interval_coefficients(id,o+d*(start+float(step)*dt),d,dt,quadrature_fraction,context);}
                    let sigma=aggregates[step];
                    let attenuation=exp(-min(medium_optical_depth(sigma,dt),float3(REG_MEDIUM_EXP_LIMIT)));
                    incident[step]+=medium_integrated_coefficient(m.emission,sigma,dt,attenuation);
                    if(!any(m.scattering>float3(0.)) || !medium_lighting_cell(uint(step),sample_fraction,uint(Steps))) {continue;}
                    incident[step]+=medium_integrated_coefficient(m.scattering,sigma,dt,attenuation)
                        *(Shared ? medium_directional_illumination(p,d,m.anisotropy) : medium_illumination(p,d,m.anisotropy,d*dt,quadrature_fraction))*medium_lighting_weight(uint(Steps));
                }
            }
        }
        for(var step=0u;step<(Lit ? Steps : 1);step++) {
            let sigma=aggregates[step];
            let cell=MediumTransport(Lit ? incident[step] : float3(0.),exp(-min(medium_optical_depth(sigma,dt),float3(REG_MEDIUM_EXP_LIMIT))),medium_optical_depth(sigma,dt));
            source+=medium_received_source(transmission,cell.source);transmission*=cell.transmission;optical_depth+=cell.optical_depth;
        }
#endif
        start=end;
    }
    return MediumTransport(source,transmission,optical_depth);
}

public export MediumTransport medium_transport_segment(float3 o,float3 d,MediumEndpoint endpoint,float sample_fraction,uint inside_object) {
    return volume_finite<VORK_MEDIUM_STEPS,true>(o,d,endpoint,sample_fraction,inside_object,true);
}
public MediumTransport medium_transport_infinity(float3 o,float3 d,float scale,float fraction) {
    return volume_finite<VORK_MEDIUM_STEPS,true,true>(o,d,medium_infinity(),fraction,0xffffffffu,true,scale);
}
#if VORK_VOLUME_SHADOWS
public export float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint) {
    return volume_finite<VOLUME_SHADOW_SAMPLES,false>(o,d,endpoint,.5,0xffffffffu,false).transmission;
}
#endif

#if VORK_MEDIUM_BOUNDARIES > 0
public export MediumTransport medium_transport_range(float3 o,float3 d,float start,float end,float fraction) {
    medium_column_offset(start);
    return medium_transport_segment(o+d*start,d,medium_receiver(end-start),fraction,0xffffffffu);
}
#endif
