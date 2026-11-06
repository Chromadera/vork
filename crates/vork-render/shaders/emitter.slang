import orthonormal_basis;
// Physical analytic emitter geometry, independent of an integrator's random
// stream, visibility query, receiver BSDF and discrete source selection.
// PDFs are conditional solid-angle densities; radiance excludes visibility.
// Delta sources have no solid-angle density and retain their point evaluation
// in the consumer. No resources or retained tables are owned by this module.
import vork_constants;
import frame;
import material;
import variant;
__exported import light_connection;

// Frisvad/Duff orthonormal frame, independent of receiver shading.
float3 ies_bracket(StructuredBuffer<uint> words,uint base,uint count,float angle) {
    var low=0u;var high=count;
    while(low<high) {let middle=(low+high)/2u;if(asfloat(words[base+middle])<angle){low=middle+1u;}else{high=middle;}}
    let hi=min(low,count-1u);let lo=hi>0u ? hi-1u : 0u;
    let a=asfloat(words[base+lo]);let b=asfloat(words[base+hi]);
    return float3(float(lo),float(hi),hi==lo ? 0. : saturate((angle-a)/(b-a)));
}
// Refine the native atan2 with its sine/cosine residual. Native atan2's
// approximation error can exceed a narrow photometric knot interval even
// though the input direction still resolves it. Scaling avoids overflow and
// preserves poles without a scene/angular support threshold.
float emitter_angle(float y,float x) {
    let scale=max(abs(x),abs(y));if(scale==0.) {return 0.;}
    let sx=x/scale;let sy=y/scale;var angle=atan2(sy,sx);
    let sine=sin(angle);let cosine=cos(angle);
    angle+=fma(sy,cosine,-sx*sine)/(sx*cosine+sy*sine);
    return angle;
}
// Original Type C angular knots, not a proposal texture or peak-normalized map.
float emitter_profile_words(LightValues l,float3 outgoing,StructuredBuffer<uint> words) {
    let base=asuint(l.emission.w);if(base==0u) {return 1.;}
    let q=l.emission.xyz;let qw=sqrt(max(1.-dot(q,q),0.));
    let local=outgoing+2.*cross(q,cross(q,outgoing)-qw*outgoing);
    let polar=emitter_angle(length(local.xy),-local.z)*(180./REG_PI);
    let nv=words[base];let nh=words[base+1u];let vb=base+2u;let hb=vb+nv;
    if(polar<asfloat(words[vb]) || polar>asfloat(words[vb+nv-1u])) {return 0.;}
    var azimuth=emitter_angle(local.y,local.x)*(180./REG_PI);if(azimuth<0.) {azimuth+=360.;}
    let end=asfloat(words[hb+nh-1u]);
    if(end==0.) {azimuth=0.;}
    else if(end==90.) {azimuth=fmod(azimuth,180.);if(azimuth>90.) {azimuth=180.-azimuth;}}
    else if(end==180. && azimuth>180.) {azimuth=360.-azimuth;}
    let v=ies_bracket(words,vb,nv,polar);let h=ies_bracket(words,hb,nh,azimuth);let values=hb+nh;
    let a=lerp(asfloat(words[values+uint(h.x)*nv+uint(v.x)]),asfloat(words[values+uint(h.x)*nv+uint(v.y)]),v.z);
    let b=lerp(asfloat(words[values+uint(h.y)*nv+uint(v.x)]),asfloat(words[values+uint(h.y)*nv+uint(v.y)]),v.z);
    return lerp(a,b,h.z);
}
float emitter_profile<V:IVariant>(LightValues l,float3 outgoing) {
    if(!V.has_emitter_profiles) {return 1.;}
    return emitter_profile_words(l,outgoing,material_words);
}
float emitter_window(LightValues l,float3 world,float3 direction) {
    var value=1.;
    if(l.position.w>0. && l.projection.y>0.) {
        let relative=(l.position.xyz-world)/l.projection.y;let ratio=dot(relative,relative);
        value=max(1.-ratio*ratio,0.);
    }
    if(l.position.w==2.) {
        let cosine=dot(-direction,normalize(l.direction.xyz));let outer=l.direction.w;let inner=l.radiance.w;
        var cone=select(cosine>=outer,1.,0.);
        if(inner>outer) {let t=saturate((cosine-outer)/(inner-outer));cone=select(l.projection.z>0.,t*t,smoothstep(outer,inner,cosine));}
        value*=cone;
    }
    return value;
}

// The sampled direction to a light: area-uniform planes, solid-angle-uniform
// spheres and solar caps, with the corresponding first-emitter distance.
float3 emitter_cone_direction(float3 axis, float one_minus_cosine, float u, float v) {
    let height=u*one_minus_cosine;let radius=sqrt(max(height*(2.-height),0.));
    return basis_direction(axis,float3(radius*cos(REG_TAU*v),radius*sin(REG_TAU*v),1.-height));
}
float3 emitter_direction(LightValues l, float3 world, float2 u, out float distance) {
    var axis=normalize(-l.direction.xyz);distance=REG_FP32_MAX;
    if (l.position.w==0. && l.params.y>0.) {
        let sine=sin(l.params.y*.5);
        axis=emitter_cone_direction(axis,2.*sine*sine,u.x,u.y);
    }
    if (l.position.w>0.) {
        var point=l.position.xyz;let delta=point-world;distance=length(delta);axis=distance>0. ? delta/distance : float3(0.);
        if (l.position.w==3.) {
            var sample=u.xy-.5;
            if (l.emitter_u.w>0.) {let r=sqrt(u.x)*.5;let phi=REG_TAU*u.y;sample=r*float2(cos(phi),sin(phi));}
            point+=sample.x*l.emitter_u.xyz+sample.y*l.emitter_v.xyz;
            distance=length(point-world);axis=distance>0. ? (point-world)/distance : float3(0.);
        } else if (l.params.y>0. && distance>l.params.y) {
            let ratio=l.params.y/distance;let square=ratio*ratio;
            axis=emitter_cone_direction(axis,square/(1.+sqrt(1.-square)),u.x,u.y);
            let along=dot(delta,axis);let perpendicular=cross(delta,axis);
            distance=along-sqrt(max(l.params.y*l.params.y-dot(perpendicular,perpendicular),0.));
        }
    }
    return axis;
}
// A light's conditional solid-angle PDF and emitted radiance. The virtual
// rectangle/ellipse is shared with raster LTC; points/spots are physical
// emitting spheres and suns have uniform radiance over their angular cap.
// Both direct strategies select the SAME light, so its discrete probability
// cancels in their heuristic; both estimates still divide by that probability.
struct EmitterEvaluation { float3 radiance; float pdf; float distance; };
bool emitter_delta(LightValues l) {return l.position.w!=3. && l.params.y==0.;}
EmitterEvaluation emitter_eval_li<V:IVariant>(LightValues l, float3 world, float3 direction) {
    EmitterEvaluation result={};if (emitter_delta(l)) {return result;}
    if (l.position.w==0.) {
        let axis=normalize(-l.direction.xyz);let sine=sin(l.params.y);
        let perpendicular=cross(axis,direction);
        if (dot(axis,direction)<=0. || dot(perpendicular,perpendicular)>sine*sine) {return result;}
        let half_sine=sin(l.params.y*.5);
        result.pdf=1./(4.*REG_PI*half_sine*half_sine);
        result.radiance=l.radiance.rgb*emitter_profile<V>(l,-direction)/(REG_PI*sine*sine);result.distance=REG_FP32_MAX;
        return result;
    }
    if (l.position.w<3.) {
        let delta=l.position.xyz-world;let squared=dot(delta,delta);let radius=l.params.y;
        // A physical outward-emitting sphere does not illuminate its interior.
        if (squared<=radius*radius) {return result;}
        let along=dot(delta,direction);let perpendicular=cross(delta,direction);
        let discriminant=radius*radius-dot(perpendicular,perpendicular);
        if (along<=0. || discriminant<0.) {return result;}
        let ratio=radius*radius/squared;
        result.pdf=(1.+sqrt(1.-ratio))/(REG_TAU*ratio);
        result.radiance=l.radiance.rgb/(4.*REG_PI*REG_PI*radius*radius);
        result.radiance*=emitter_window(l,world,direction)*emitter_profile<V>(l,-direction);
        result.distance=along-sqrt(discriminant);return result;
    }
    let center=l.position.xyz-world;
    if(dot(l.direction.xyz,-center)<=0.) {return result;}
    let u=l.emitter_u.xyz*.5;let v=l.emitter_v.xyz*.5;
    let ellipse=l.emitter_u.w>0.;
    let normal=cross(u,v);let square=dot(normal,normal);if (square<=0.) {return result;}
    let denominator=dot(normal,direction);if (denominator==0.) {return result;}
    let distance=dot(normal,center)/denominator;if (distance<=0.) {return result;}
    let q=distance*direction-center;
    let uu=dot(u,u);let uv=dot(u,v);let vv=dot(v,v);
    let qu=dot(q,u);let qv=dot(q,v);
    let coordinate=float2(qu*vv-qv*uv,qv*uu-qu*uv)/square;
    if (ellipse ? dot(coordinate,coordinate)>1. : any(abs(coordinate)>1.)) {return result;}
    let area=4.*sqrt(square)*(ellipse ? REG_PI*.25 : 1.);
    result.pdf=distance*distance/(area*abs(denominator)/sqrt(square));
    result.radiance=l.radiance.rgb*emitter_profile<V>(l,-direction)/(REG_PI*area);
    result.distance=l.position.w>0. ? distance : REG_FP32_MAX;
    if (l.position.w>0.) {result.radiance*=emitter_window(l,world,direction);}
    return result;
}

// Conditional source measure. Discrete source-selection probability remains in
// the consumer, never folded into these shape PDFs. Delta radiance is the
// integrated connection weight (a unit probability mass, no sr^-1 density).
LightConnection emitter_sample_li<V:IVariant>(LightValues l,uint identity,float3 world,float2 u,uint sample_index=0u) {
    LightConnection sample={};sample.identity=identity;sample.sample_index=sample_index;sample.delta=emitter_delta(l);
    sample.direction=emitter_direction(l,world,u,sample.distance);
    if(l.position.w>0.) {sample.endpoint=world+sample.direction*sample.distance;}
    if(sample.delta) {
        sample.radiance=l.radiance.rgb*emitter_profile<V>(l,-sample.direction);
        if(l.position.w>0.) {
            let squared=sample.distance*sample.distance;
            let denominator=4.*REG_PI*squared;
            sample.radiance=denominator>0. ? sample.radiance/denominator*emitter_window(l,world,sample.direction) : float3(0.);
        }
    } else {
        let value=emitter_eval_li<V>(l,world,sample.direction);
        sample.radiance=value.radiance;sample.pdf=value.pdf;
    }
    return sample;
}

// Analytic secondary-response approximation: one central source connection.
// Shape construction still belongs to the physical sampler (including the
// first sphere surface), independently of this consumer's quadrature policy.
LightConnection emitter_center_li<V:IVariant>(LightValues light,uint identity,float3 world,uint sample_index=0u) {
    let u=light.position.w==3. && light.emitter_u.w==0. ? float2(.5) : float2(0.);
    return emitter_sample_li<V>(light,identity,world,u,sample_index);
}

// Distant declared emitters have no finite surface in the ray hierarchy.
// Camera escapes see their ordinary shape and radiance, including bent exits.
float3 camera_distant_lights(Frame f,StructuredBuffer<Light> source_lights,float3 direction) {
    var radiance=float3(0.);
    for(uint i=0u;i<uint(frame_light_count(f));i++) {
        let l=specialized_light<Unspecialized>(source_lights[i]);
        if(l.position.w!=0. || (source_lights[i].shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]&REG_RAY_CAMERA)==0u) {continue;}
        radiance+=emitter_eval_li<Unspecialized>(l,float3(0.),direction).radiance;
    }
    return radiance;
}
