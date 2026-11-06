#ifndef VORK_MEDIUM_NO_ANALYTIC_LIGHTS
#define VORK_MEDIUM_NO_ANALYTIC_LIGHTS 0
#endif
#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
#ifndef VORK_VOLUME_SLICES
#define VORK_VOLUME_SLICES 64
#endif
import vork_constants;
import vork_volume_settings;
import vork_prelude;
import vork_light_falloff;
import vork_sampling;
import medium_environment;
internal extern Frame medium_frame();
internal extern Light medium_light(uint index);
internal extern uint medium_light_list(float3 position,out uint base);
internal extern uint medium_light_index(uint base,uint next);
internal extern float3 medium_visibility(float3 position,uint light_index);
#ifdef VORK_MEDIUM_MESH_SOURCES
internal extern float3 medium_mesh_illumination(float3 position,float3 ray,float g);
#endif
#if VORK_VOLUME_SHADOWS
internal extern float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint);
#endif
// The consuming resolve pass exports these resource accessors and medium_transport.
// `export` definitions satisfy these link-time `extern` declarations.
internal extern uint2 medium_size();
__exported import vork_medium_point;
__exported import vork_medium_types;
internal extern MediumTransport medium_transport_segment(float3 o,float3 d,MediumEndpoint endpoint,float fraction,uint inside_object);
#ifdef VORK_FROXEL
import vork_froxel;
#endif
__exported import vork_medium_visibility;
// The finite source cell supplies its world-metre footprint, not a screen-space
// blur radius. All strata count, including fully occluded queries. The
// exposure fraction is inherited unchanged, so seeking requires no history.
// Cap per-cell extra work in previews and amortize a 256-query source budget
// over the exposure. At 256+ samples the original point query is sufficient.
public uint medium_visibility_taps(uint count) {
    count=max(count,1u);
    return min(16u,min(max(4u,count),1u+255u/count));
}
public float3 medium_source_footprint(float3 position,float3 footprint,float fraction,uint light,
    float3 width_x=float3(0.),float3 width_y=float3(0.),uint2 pixel=uint2(0u),uint index=0u,uint count=1u,
    float3 ray=float3(0.,0.,1.),float g=0.) {
    let taps=medium_visibility_taps(count);
    let source=medium_light(light);
    if(all(footprint==float3(0.)) || taps==1u) {
        let falloff=light_falloff(position,source.position,source.direction,source.radiance.w,source.projection.y,source.projection.z>0.);
        let weight=falloff.punctual*medium_phase_weight(falloff.axis,ray,g);
        return weight>0. ? weight*medium_visibility(position,light) : float3(0.);
    }
    var reference=float3(0.);var estimate=float3(0.);
    let lateral=any(width_x!=float3(0.)) || any(width_y!=float3(0.));
    var sequence=sample_2d_sequence(pixel,index*taps,max(count,1u)*taps,REG_SAMPLING_STREAM_VOLUME+light+1u);
    [loop] for(uint tap=0u;tap<taps;tap++) {
        let offset=(float(tap)+fraction)/float(taps)-fraction;
        let uv=lateral ? sample_2d_next(sequence)-.5 : float2(0.);
        let point=position+footprint*offset+width_x*uv.x+width_y*uv.y;
        let falloff=light_falloff(point,source.position,source.direction,source.radiance.w,source.projection.y,source.projection.z>0.);
        let weight=falloff.punctual*medium_phase_weight(falloff.axis,ray,g);
        // Outside a source's cone/range there is no incident mass. A visibility
        // query there may report clear, which must not illuminate an in-cone
        // point. Blocked queries inside the source retain their full mass.
        let incident=weight>0. ? weight*medium_visibility(point,light) : float3(0.);
        if(tap==0u) {reference=incident;}
        // Include blocked and out-of-cone strata in the complete footprint
        // measure. Differences preserve a constant incident RGB field exactly.
        estimate+=incident-reference;
    }
    return reference+estimate/float(taps);
}
// Finite intervals and the tail closures share this lighting/shadow graph.
// Preserve that boundary through native compilation instead of cloning the
// complete light connection evaluator into every transport call site.
[noinline]
public float3 medium_illumination(float3 position,float3 ray,float g,float3 footprint=float3(0.),float fraction=.5) {
#ifdef VORK_BOUNDED_MEDIUM
    var incoming=medium_shared_illumination(position,ray,g);
#ifdef VORK_MEDIUM_MESH_SOURCES
    incoming+=medium_mesh_illumination(position,ray,g);
#endif
    return incoming;
#else
    var width_x=float3(0.);var width_y=float3(0.);var pixel=uint2(0u);
#ifdef VORK_VOLUME_LIGHTING
    // A box in view-depth coordinates: lateral queries cannot move a source
    // behind the cell's far plane. Apply/terminal and direct interval consumers
    // retain only their clipped longitudinal footprint.
    let f=medium_frame();
    let right=float3(f.view[0].x,f.view[1].x,f.view[2].x);
    let up=float3(f.view[0].y,f.view[1].y,f.view[2].y);
    let forward=-float3(f.view[0].z,f.view[1].z,f.view[2].z);
    let centre=position+footprint*(.5-fraction);
    let depth=f.flags.x>.5 ? 1. : max(dot(centre-f.eye.xyz,forward),0.);
    // One exposure stratum's projected area. Shrinking each axis by sqrt(N)
    // makes this a convergent estimator rather than permanent shaft blurring.
    let width=2.*float(VORK_VOLUME_TILE)*depth*rsqrt(max(f.temporal.y,1.))/(float2(medium_size())*f.projection.yx);
    width_x=right*width.x;width_y=up*width.y;
    let clip=mul(float4(centre,1.),f.vp);
    pixel=uint2(max((clip.xy/clip.w*float2(.5,-.5)+.5)*float2(medium_size()),float2(0.)))/uint(VORK_VOLUME_TILE);
#endif
#ifdef VORK_MEDIUM_ENVIRONMENT
    var incoming=medium_environment_illumination(position,ray,g);
#else
    var incoming=select((medium_frame().medium.w&1u)!=0u,medium_frame().radiance.rgb,float3(0.));
    if ((medium_frame().medium.w&1u)!=0u) {let f=medium_frame();incoming+=f.environment_mean.rgb*f.environment_color.rgb*f.environment.x;}
#endif
    uint base=0u;uint count=0u;
#if !VORK_MEDIUM_NO_ANALYTIC_LIGHTS
    count=medium_light_list(position,base);
#endif
    for(var next=0u;next<count;next++) {
        let i=medium_light_index(base,next);
        if ((medium_light(i).shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]&REG_RAY_VOLUME)==0u || all(medium_light(i).radiance.rgb==float3(0.))) {continue;}
        let falloff=light_falloff(position,medium_light(i).position,medium_light(i).direction,medium_light(i).radiance.w,medium_light(i).projection.y,medium_light(i).projection.z>0.);
        let axis=falloff.axis;
        {
            var value=medium_light(i).radiance.rgb*medium_source_footprint(position,footprint,fraction,i,
                width_x,width_y,pixel,uint(medium_frame().temporal.x),uint(medium_frame().temporal.y),ray,g);
#if VORK_VOLUME_SHADOWS
            let endpoint=medium_light(i).position.w==0. ? medium_infinity() : medium_emitter(length(medium_light(i).position.xyz-position));
            value*=medium_transmittance(position+axis*REG_POSITIVE_EPSILON,axis,endpoint);
#endif
            value=medium_limit_light(value);
#ifdef VORK_MEDIUM_SAMPLED_ANALYTIC_LIGHT
            // Inverse proposal probability follows the per-source clamp.
            value*=medium_analytic_weight();
#endif
            incoming+=value;
        }
    }
#ifdef VORK_MEDIUM_MESH_SOURCES
    incoming+=medium_mesh_illumination(position,ray,g);
#endif
    return incoming;
#endif
}

// Probe transport precedes Volume; camera tails, transmission and reflections
// consume the current visibility field. Keep a single incident-light graph.
[noinline]
public float3 medium_directional_illumination(float3 position,float3 ray,float g) {
#if defined(VORK_FROXEL) || defined(VORK_BOUNDED_MEDIUM) || defined(VORK_REFLECTION_MEDIUM)
    var incoming=medium_shared_illumination(position,ray,g);
#ifdef VORK_MEDIUM_MESH_SOURCES
    incoming+=medium_mesh_illumination(position,ray,g);
#endif
    return incoming;
#else
    return medium_illumination(position,ray,g);
#endif
}

// Secondary camera samples estimate the same retained lighting quadrature by
// selecting one of its cells, with inverse selection-probability weighting.
// Extinction still integrates every cell. Other consumers retain the complete
// quadrature. No segment length, medium domain or ray depth is truncated.
public uint medium_lighting_begin(float fraction,uint count) {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
    return min(uint(fraction*float(count)),count-1u);
#else
    return 0u;
#endif
}
public uint medium_lighting_end(float fraction,uint count) {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
    return medium_lighting_begin(fraction,count)+1u;
#else
    return count;
#endif
}
public float medium_lighting_weight(uint count) {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
    return float(count);
#else
    return 1.;
#endif
}
public float medium_quadrature_fraction(float fraction,uint count) {
#ifdef VORK_MEDIUM_JITTERED_LIGHTING
    // A uniform coordinate factors into an independent cell and local offset.
    // Keep stochastic within-cell quadrature when selecting just one lit cell.
    return frac(fraction*float(count));
#elif defined(VORK_MEDIUM_SAMPLED_LIGHTING)
    return .5;
#else
    return fraction;
#endif
}
public bool medium_lighting_cell(uint cell,float fraction,uint count) {
    return cell>=medium_lighting_begin(fraction,count) && cell<medium_lighting_end(fraction,count);
}

// World closure after the domain iterator has exhausted mesh intersections.
// No camera clipping distance cuts off the physical world.
internal extern MediumSample medium_world_coefficients(float3 position);
internal extern float medium_world_tail_start(float3 o,float3 d);
internal extern MediumSample medium_world_tail_coefficients(float3 o,float3 d);
public MediumTransport medium_world_constant_tail<let Lit:bool>(float3 o,float3 d,float fraction,MediumSample m) {
    MediumExtinction sigma={};medium_extinction_add(sigma,m);
    let positive=sigma.scale>float3(0.);
    var source=Lit ? select(positive,medium_equilibrium(m.emission,sigma),
        select(m.emission>float3(0.),float3(asfloat(0x7f800000u)),float3(0.))) : float3(0.);
    let scattering=medium_equilibrium(m.scattering,sigma);let log_sigma=medium_log_extinction(sigma);
    let transmission=select(positive,float3(0.),float3(1.));
    if(Lit && any(m.scattering>float3(0.))) {
        // Spectral exponential free flights integrate constant incident light
        // exactly. Equal extinction shares one lighting evaluation per sample.
        for(uint i=medium_lighting_begin(fraction,VORK_VOLUME_SLICES);i<medium_lighting_end(fraction,VORK_VOLUME_SLICES);i++) {
            let u=clamp((float(i)+medium_quadrature_fraction(fraction,VORK_VOLUME_SLICES))/float(VORK_VOLUME_SLICES),1.e-7,1.-1.e-7);
            let optical=-log(1.-u);
            if(medium_extinction_equal(sigma,0u,1u) && medium_extinction_equal(sigma,0u,2u)) {
                let light=medium_directional_illumination(o+d*medium_optical_distance(sigma,0u,optical),d,m.anisotropy);
                source+=scattering*light*medium_lighting_weight(VORK_VOLUME_SLICES)/float(VORK_VOLUME_SLICES);
            } else {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
                let draw=min(u*3.,2.9999998);let channel=uint(draw);
                if(sigma.scale[channel]<=0.) {continue;}
                let optical_draw=-log(1.-frac(draw));
                let distance=medium_optical_distance(sigma,channel,optical_draw);
                let log_pdf=select(positive,log_sigma-medium_relative_optical(log_sigma,channel,optical_draw),float3(-REG_FP32_MAX));
                let weight=medium_spectral_weights(log_pdf);
                let light=medium_directional_illumination(o+d*distance,d,m.anisotropy);
                source+=scattering*weight*light*medium_lighting_weight(VORK_VOLUME_SLICES)/float(VORK_VOLUME_SLICES);
#else
                [unroll] for(uint channel=0u;channel<3u;channel++) {
                    if(m.scattering[channel]>0.) {
                        let light=medium_directional_illumination(o+d*medium_optical_distance(sigma,channel,optical),d,m.anisotropy);
                        source[channel]+=scattering[channel]*light[channel]*medium_lighting_weight(VORK_VOLUME_SLICES)/float(VORK_VOLUME_SLICES);
                    }
                }
#endif
            }
        }
    }
    return MediumTransport(source,transmission,float3(0.));
}
internal extern void medium_world_exponential(out float3 absorption,out float3 scattering,out float3 gradient);
public MediumTransport medium_world_exponential_tail<let Lit:bool>(float3 o,float3 d,float fraction) {
    float3 absorption,scattering,gradient;medium_world_exponential(absorption,scattering,gradient);
    MediumExtinction amplitude={};medium_extinction_add(amplitude,MediumSample(absorption,scattering,float3(0.),0.));
    let log_density=dot(gradient,o);let slope=dot(gradient,d);
    let tau=medium_exponential_tau(absorption,log_density,slope,0.,true)+medium_exponential_tau(scattering,log_density,slope,0.,true);
    let scattering_weight=medium_equilibrium(scattering,amplitude);let log_amplitude=medium_log_extinction(amplitude);
    let transmission=exp(-tau);var source=float3(0.);
    if(Lit) {
        let m=medium_world_coefficients(float3(0.));
        for(uint channel=0u;channel<3u;channel++) {
            let sigma=amplitude.scale[channel];let emission=m.emission[channel];
            if(sigma==0. || slope<0.) {
                if(emission!=0.) {source[channel]=emission*asfloat(0x7f800000u);}
            }
            if(sigma==0.) {continue;}
            // Equal extinction has the same inverse optical flight. Share its
            // directional light query while retaining each channel's source.
            var duplicate=false;
            for(uint previous=0u;previous<channel;previous++) {duplicate=duplicate || medium_extinction_equal(amplitude,previous,channel);}
            if(duplicate) {continue;}
            let group=bool3(medium_extinction_equal(amplitude,0u,channel),medium_extinction_equal(amplitude,1u,channel),medium_extinction_equal(amplitude,2u,channel));
            let group_scattering=select(group,scattering_weight,float3(0.));
            let group_emission=select(group,m.emission,float3(0.));
            let log_sigma=log_amplitude[channel]+log_density;
            let mass=medium_optical_mass(tau[channel]);
            if(mass==0.) {continue;}
            // Emission has no expensive incident-light query. Retain its full
            // optical quadrature even when scattering selects one lit cell.
            for(uint i=0u;i<VORK_VOLUME_SLICES;i++) {
                let u=clamp((float(i)+medium_quadrature_fraction(fraction,VORK_VOLUME_SLICES))/float(VORK_VOLUME_SLICES),1.e-7,1.-1.e-7);
                let optical=medium_optical_quantile(u*mass);
                var distance=0.;
                if(slope==0.) {distance=exp(log(optical)-log_sigma);}
                else {
                    let log_ratio=log(abs(slope))+log(optical)-log_sigma;
                    if(slope<0.) {distance=-medium_optical_quantile(min(optical/tau[channel],1.-1.e-7))/slope;}
                    else {distance=(max(log_ratio,0.)+log(1.+exp(-abs(log_ratio))))/slope;}
                }
#ifndef VORK_MEDIUM_SAMPLED_LIGHTING
                if(any(group_scattering>float3(0.)) && mass>0.) {
                    let incoming=medium_directional_illumination(o+d*distance,d,m.anisotropy);
                    source+=select(group,mass*group_scattering*incoming*medium_lighting_weight(VORK_VOLUME_SLICES)/float(VORK_VOLUME_SLICES),float3(0.));
                }
#endif
                if(any(group_emission!=float3(0.)) && slope>=0.) {
                    let log_event_sigma=log_sigma+slope*distance;
                    source+=select(group_emission!=float3(0.),mass*medium_exponential_coefficient(group_emission,-log_event_sigma)/float(VORK_VOLUME_SLICES),float3(0.));
                }
            }
        }
    }
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
    if(Lit && any(scattering>float3(0.))) {
        let m=medium_world_coefficients(float3(0.));
        let positive=amplitude.scale>float3(0.);
        let mass=float3(medium_optical_mass(tau.x),medium_optical_mass(tau.y),medium_optical_mass(tau.z));
        for(uint i=medium_lighting_begin(fraction,VORK_VOLUME_SLICES);i<medium_lighting_end(fraction,VORK_VOLUME_SLICES);i++) {
            let u=clamp((float(i)+medium_quadrature_fraction(fraction,VORK_VOLUME_SLICES))/float(VORK_VOLUME_SLICES),1.e-7,1.-1.e-7);
            let draw=min(u*3.,2.9999998);let channel=uint(draw);let sigma=amplitude.scale[channel];
            if(sigma<=0. || mass[channel]<=0.) {continue;}
            let optical=medium_optical_quantile(frac(draw)*mass[channel]);
            let log_sigma=log_amplitude[channel]+log_density;
            var distance=0.;
            if(slope==0.) {distance=exp(log(optical)-log_sigma);}
            else {
                let log_ratio=log(abs(slope))+log(optical)-log_sigma;
                if(slope<0.) {distance=-medium_optical_quantile(min(optical/tau[channel],1.-1.e-7))/slope;}
                else {distance=(max(log_ratio,0.)+log(1.+exp(-abs(log_ratio))))/slope;}
            }
            let relative_optical=medium_relative_optical(log_amplitude,channel,optical);
            let log_pdf=select(positive && mass>float3(0.),log_amplitude-relative_optical-log(select(mass>float3(0.),mass,float3(1.))),float3(-REG_FP32_MAX));
            let weight=medium_spectral_weights(log_pdf);
            let incoming=medium_directional_illumination(o+d*distance,d,m.anisotropy);
            source+=mass*scattering_weight*weight*incoming*medium_lighting_weight(VORK_VOLUME_SLICES)/float(VORK_VOLUME_SLICES);
        }
    }
#endif
    return MediumTransport(source,transmission,float3(0.));
}
// Both world-tail partitions consume the finite service's coefficient weights.
// Lighting is evaluated once per selected stratum, never inside coefficient work.
MediumTransport medium_world_cell<let Lit:bool>(MediumSample m,float3 p,float3 d,float dt,uint cell,float fraction) {
    MediumExtinction sigma={};medium_extinction_add(sigma,m);
    let tau=medium_optical_depth(sigma,dt);
    let attenuation=exp(-min(tau,float3(REG_MEDIUM_EXP_LIMIT)));
    var source=float3(0.);
    if(Lit) {
        source=medium_integrated_coefficient(m.emission,sigma,dt,attenuation);
        if(any(m.scattering>float3(0.)) && medium_lighting_cell(cell,fraction,VORK_VOLUME_SLICES)) {
            source+=medium_integrated_coefficient(m.scattering,sigma,dt,attenuation)
                *medium_directional_illumination(p,d,m.anisotropy)*medium_lighting_weight(VORK_VOLUME_SLICES);
        }
    }
    return MediumTransport(source,attenuation,tau);
}
public MediumTransport medium_world_tail<let Lit:bool>(float3 o,float3 d,float scale,float fraction) {
    if((medium_frame().medium.w&1u)==0u) return MediumTransport(float3(0.),float3(1.),float3(0.));
    let start=medium_world_tail_start(o,d);
    if(start==-3.) return medium_world_exponential_tail<Lit>(o,d,fraction);
    if(Lit && start==-2.) return MediumTransport(float3(asfloat(0x7f800000u)),float3(1.),float3(0.));
    if(start>=0.) {
        let tail=medium_world_constant_tail<Lit>(o+d*start,d,fraction,medium_world_tail_coefficients(o,d));
        if(start==0.) return tail;
        // The domain iterator has exhausted bounded mesh intervals. Only world
        // coefficients belong in this asymptotic closure.
        var prefix=MediumTransport(float3(0.),float3(1.),float3(0.));
        // A graded finite partition resolves the local free path even when
        // the proven exterior is astronomically distant. Work in log lengths
        // so extinction*distance need not fit in a float. With zero extinction,
        // use the finite interval's resolution scale instead of a free path.
        let initial=medium_world_coefficients(o);
        MediumExtinction sigma0={};medium_extinction_add(sigma0,initial);
        let logs=medium_log_extinction(sigma0);let log_extinction=max(logs.x,max(logs.y,logs.z));
        let log_distance=log(start);
        let log_ratio=max(log(float(VORK_VOLUME_SLICES)),log_extinction+log_distance);
        let span=log_ratio+log(1.+exp(-log_ratio));
        let log_scale=log_distance-log_ratio;
        var previous=0.;
        for(uint i=0u;i<VORK_VOLUME_SLICES;i++) {
            let q=span*float(i+1u)/float(VORK_VOLUME_SLICES);
            let next=select(i+1u==VORK_VOLUME_SLICES,start,exp(log_scale+q)*(1.-exp(-q)));
            let dt=next-previous;
            // Gaussian coefficient quadrature resolves the finite transition
            // to an extremely distant floor without random residual weights.
            // Lighting retains one spatial evaluation per stratum.
            let p=o+d*(previous+0.5*dt);
            var m=medium_world_coefficients(p);
            let a=medium_world_coefficients(o+d*(previous+0.1127016654*dt));
            let b=medium_world_coefficients(o+d*(previous+0.8872983346*dt));
            m.absorption=a.absorption*(5./18.)+m.absorption*(4./9.)+b.absorption*(5./18.);
            m.scattering=a.scattering*(5./18.)+m.scattering*(4./9.)+b.scattering*(5./18.);
            m.emission=a.emission*(5./18.)+m.emission*(4./9.)+b.emission*(5./18.);
            medium_append(prefix,medium_world_cell<Lit>(m,p,d,dt,i,fraction));
            previous=next;
        }
        return MediumTransport(prefix.source+medium_received_source(prefix.transmission,tail.source),prefix.transmission*tail.transmission,float3(0.));
    }
    // Rational compactification maps [0,1) to [0,infinity). Stratified coefficient
    // quadrature remains approximate, like the finite varying-medium integrator.
    var result=MediumTransport(float3(0.),float3(1.),float3(0.));
    for(uint i=0u;i<VORK_VOLUME_SLICES;i++) {
        let u=clamp((float(i)+(Lit ? medium_quadrature_fraction(fraction,VORK_VOLUME_SLICES) : fraction))/float(VORK_VOLUME_SLICES),1.e-7,1.-1.e-7);
        let remaining=1.-u;let distance=scale*u/remaining;
        let dt=scale/(remaining*remaining*float(VORK_VOLUME_SLICES));
        let p=o+d*distance;
        medium_append(result,medium_world_cell<Lit>(medium_world_coefficients(p),p,d,dt,i,fraction));
    }
    return result;
}
