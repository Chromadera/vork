// Free-flight estimator over the shared point/domain service. Majorants and
// real/null-event weights are estimator state; all distances are world metres.
import vork_constants;
// Outward addition for nonnegative extinction bounds. Promote subnormals
// before arithmetic so optional GPU flush-to-zero cannot erase a bound.
float medium_majorant_normalize(float value) {
    let bits=asuint(value);
    return bits>0u && bits<0x00800000u ? asfloat(0x00800000u) : value;
}
float medium_majorant_add(float a,float b) {
    a=medium_majorant_normalize(a);b=medium_majorant_normalize(b);
    if(a==0.) {return b;}if(b==0.) {return a;}
    let sum=a+b;return isfinite(sum) ? asfloat(asuint(sum)+1u) : sum;
}
float medium_sample_majorant(float3 absorption,float3 scattering) {
    return max(medium_majorant_add(absorption.x,scattering.x),max(
        medium_majorant_add(absorption.y,scattering.y),
        medium_majorant_add(absorption.z,scattering.z)));
}
struct PathFreeFlight {
    float distance;
    float3 weight; // RGB transmittance divided by the sampled distance measure.
    bool event;
};
PathFreeFlight medium_free_flight(float3 extinction,float limit,float2 u) {
    let channel=min(uint(u.x*3.),2u);
    let sigma=extinction[channel];
    let distance=sigma>0. ? -log(1.-u.y)/sigma : REG_FP32_MAX;
    PathFreeFlight result;result.distance=min(distance,limit);result.event=distance<limit;
    let transmission=exp(-extinction*result.distance);
    // Uniform spectral-channel mixture, including zero-extinction channels.
    // Collision density and boundary survival are different measures.
    let density=result.event ? extinction*transmission : transmission;
    let pdf=(density.x+density.y+density.z)/3.;
    result.weight=pdf>0. ? transmission/pdf : float3(0.);
    return result;
}

import vork_medium_point;
// Inverse integrated exponential extinction, in world metres. The small-ratio
// expansion uses a precision-derived switch, not a transport distance cutoff.
float medium_exponential_distance(float amplitude,float log_density,float slope,float optical) {
    if(amplitude==0.) {return REG_FP32_MAX;}
    let log_sigma=log(amplitude)+log_density;
    let log_distance=log(optical)-log_sigma;
    if(slope==0.) {return exp(log_distance);}
    let log_ratio=log(abs(slope))+log_distance;
    if(log_ratio<log(sqrt(REG_FP32_EPSILON))) {
        let ratio=exp(log_ratio);
        return exp(log_distance)*(1.-sign(slope)*ratio*.5);
    }
    if(slope<0.) {
        if(log_ratio>=0.) {return REG_FP32_MAX;}
        return log(1.-exp(log_ratio))/slope;
    }
    return (max(log_ratio,0.)+log(1.+exp(-abs(log_ratio))))/slope;
}
// A constant certified finite-domain majorant plus a compiler-proven world
// exponential. Invert the summed optical depth to sample the full domain;
// no global density bound or finite enclosure is needed for the world.
float medium_exponential_candidate(float constant,float amplitude,float log_density,float slope,float optical,float limit) {
    let world=medium_exponential_distance(amplitude,log_density,slope,optical);
    let uniform=constant>0. ? optical/constant : REG_FP32_MAX;
    if(constant==0.) {return min(world,limit);}
    if(amplitude==0.) {return min(uniform,limit);}
    let total=constant*limit+medium_exponential_tau(amplitude,log_density,slope,limit,limit==REG_FP32_MAX);
    if(total<=optical) {return limit;}
    var low=0.;var high=min(min(world,uniform),limit);
    // The positive float interval strictly shrinks. Stop only when its endpoints
    // are adjacent at FP32 precision, not after a fixed iteration count.
    for(;;) {
        let middle=low+(high-low)*.5;
        if(middle==low || middle==high) {return high;}
        let tau=constant*middle+medium_exponential_tau(amplitude,log_density,slope,middle,false);
        if(tau<optical) {low=middle;} else {high=middle;}
    }
}
