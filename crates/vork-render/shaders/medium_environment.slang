#ifdef VORK_MEDIUM_ENVIRONMENT
#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
// Incident World radiance uses the same directional connection measure as path
// transport, including the phase delta endpoints and infinite optical distance.
import vork_prelude;
import vork_constants;
import vork_medium_types;
import vork_ray_sampling;
import vork_sampling;
import vork_medium_point;
internal extern Frame medium_frame();
internal extern float3 medium_environment_radiance(float3 direction);
#if VORK_VOLUME_SHADOWS
internal extern float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint);
#endif
public float3 medium_environment_illumination(float3 position,float3 ray,float g) {
    let f=medium_frame();if((f.medium.w&1u)==0u) {return float3(0.);}
    let pixel=uint2(asuint(position.x)^asuint(position.z),asuint(position.y));
    let u=sample_2d(pixel,uint(f.temporal.x),max(uint(f.temporal.y),1u),REG_SAMPLING_STREAM_MEDIUM_ENVIRONMENT);
    let phase=medium_phase_sample(g,u.x);let sine=sqrt(max(1.-phase.cosine*phase.cosine,0.));let phi=2.*REG_PI*u.y;
    let direction=mul(float3(sine*cos(phi),sine*sin(phi),phase.cosine),ray_basis(ray));
    var transmission=float3(1.);
#if VORK_VOLUME_SHADOWS
    transmission=medium_transmittance(position,direction,medium_infinity());
    if(!any(transmission>float3(0.))) {return float3(0.);}
#endif
    return transmission*medium_environment_radiance(direction);
}
#endif
