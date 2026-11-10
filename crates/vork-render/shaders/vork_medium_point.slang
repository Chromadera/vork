// Point coefficients and segment numerics for sampled path transport.
// No lighting, camera, shadow, or attachment interface.
import vork_constants;
public extern uint medium_word(uint index);
public struct MediumTransportContext {
    public uint input_base;
    public float4x4 inverse;
};
// A point program binding; discovery is owned by the domain service.
public struct MediumInstance {
    public uint program;
    public uint object;
    public MediumTransportContext context;
};
public struct MediumSample { public float3 absorption; public float3 scattering; public float3 emission; public float anisotropy; };
// Rescale a quotient into the normal reciprocal range before division. Native
// GPU division may flush 1/denominator even when the final ratio is ordinary.
public float medium_ratio(float numerator,float denominator) {
    let exponent=int((asuint(denominator)>>23u)&255u)-127;
    let factor=asfloat(uint(127-exponent/2)<<23u);
    precise float scaled_numerator=numerator*factor;
    precise float scaled_denominator=denominator*factor;
    return scaled_numerator/scaled_denominator;
}
public float3 medium_ratio(float3 numerator,float3 denominator) {
    return float3(medium_ratio(numerator.x,denominator.x),medium_ratio(numerator.y,denominator.y),medium_ratio(numerator.z,denominator.z));
}
// Extinction stays normalized until optical depth is formed. Individually finite
// overlapping coefficients may have an unrepresentable sum, while source ratios
// and the integrated radiance remain finite. Path's coefficient API is unchanged.
public struct MediumExtinction {
    public float3 value;
    public float3 scale;
};
public float3 medium_optical_depth(MediumExtinction sum,float distance) {
    let sigma=sum.scale*sum.value;
    // Preserve tiny representable optical depths when scale*distance alone
    // would underflow. Only an overflowing coefficient sum needs reordered math.
    return select(isfinite(sigma),sigma*distance,(sum.scale*distance)*sum.value);
}
// Infinity is a transport endpoint, not the largest finite FP32 distance.
// Every positive extinction closes its infinite tail, however thin it is.
public float3 medium_infinite_transmittance(float3 extinction) {
    return select(extinction>float3(0.),float3(0.),float3(1.));
}
// Equilibrium source per unit extinction; the same ratio closes infinite tails.
public float3 medium_equilibrium(float3 coefficient,MediumExtinction sum) {
    let scale=select(sum.scale>float3(0.),sum.scale,float3(1.));
    let sigma=select(sum.value>float3(0.),sum.value,float3(1.));
    let raw=sum.scale*sum.value;
    let denominator=select(raw>float3(0.),raw,float3(1.));
    return select(isfinite(raw),medium_ratio(coefficient,denominator),(medium_ratio(coefficient,scale))/sigma);
}
public float3 medium_integrated_coefficient(float3 coefficient,MediumExtinction sum,float distance,float3 attenuation) {
    let tau=medium_optical_depth(sum,distance);
    let x=min(tau,float3(REG_MEDIUM_INTEGRAL_SERIES_LIMIT));
    let series=1.+x*(-.5+x*(1./6.+x*(-1./24.+x*(1./120.-x/720.))));
    let raw=sum.scale*sum.value;
    let denominator=select(raw>float3(0.),raw,float3(1.));
    let ratio=medium_equilibrium(coefficient,sum);
    let mass=float3(1.)-attenuation;
    // A finite extinction sum may be smaller than one: dividing by its scale
    // first can overflow even when division by the whole sum is representable.
    // If the equilibrium itself overflows, apply the finite segment mass first.
    let integrated=select(isfinite(ratio),ratio*mass,medium_ratio(coefficient*mass,denominator));
    return select(tau<=float3(REG_MEDIUM_INTEGRAL_SERIES_LIMIT),coefficient*distance*series,integrated);
}
// Scalar extinction is a view of the same scaled coefficient algebra.
public float3 medium_integrated_coefficient(float3 coefficient,float3 sigma,float distance,float3 attenuation) {
    return medium_integrated_coefficient(coefficient,MediumExtinction(float3(1.),sigma),distance,attenuation);
}

// Exact exponential-profile optical depth, evaluated in log space. There is no
// density floor or vacuum threshold; horizontal infinity is opaque for every
// positive amplitude, even when its point value underflows at the ray origin.
// Saturate after exp: exp(log(FP32_MAX)) can itself overflow because the
// rounded logarithm lies above the exact representable endpoint.
public float medium_exponential_tau(float amplitude,float log_density,float slope,float distance,bool infinite) {
    if(amplitude==0.) {return 0.;}
    let log_sigma=log(amplitude)+log_density;
    if(infinite) {
        if(slope>=0.) {return asfloat(0x7f800000u);}
        return min(exp(log_sigma-log(-slope)),REG_FP32_MAX);
    }
    if(distance==0.) {return 0.;}
    let x=slope*distance;var log_length=log(distance);
    if(abs(x)<.001) {log_length+=log(1.+x*(.5+x*(1./6.+x/24.)));}
    else if(x>0.) {log_length=x+log(1.-exp(-x))-log(slope);}
    else {log_length=log(1.-exp(x))-log(-slope);}
    return min(exp(log_sigma+log_length),REG_FP32_MAX);
}
public float3 medium_exponential_tau(float3 amplitude,float log_density,float slope,float distance,bool infinite) {
    return float3(medium_exponential_tau(amplitude.x,log_density,slope,distance,infinite),
        medium_exponential_tau(amplitude.y,log_density,slope,distance,infinite),
        medium_exponential_tau(amplitude.z,log_density,slope,distance,infinite));
}
public float3 medium_exponential_coefficient(float3 amplitude,float log_density) {
    var result=float3(0.);
    for(uint c=0u;c<3u;c++) {if(amplitude[c]>0.) {result[c]=min(exp(log(amplitude[c])+log_density),REG_FP32_MAX);}}
    return result;
}

// Henyey–Greenstein response and proposal shared by all transport estimators.
public struct MediumPhaseSample {public float cosine;public float pdf;public bool delta;};
public float medium_phase(float g,float cosine) {
    if (abs(g)==1.) {return 0.;} // Directional delta, no solid-angle density.
    cosine=clamp(cosine,-1.,1.);
    let denominator=g>=0. ? (1.-g)*(1.-g)+2.*g*(1.-cosine)
        : (1.+g)*(1.+g)-2.*g*(1.+cosine);
    return (1.-g)*(1.+g)/(4.*REG_PI*denominator*sqrt(denominator));
}
public MediumPhaseSample medium_phase_sample(float g,float u) {
    MediumPhaseSample result;result.delta=abs(g)==1.;
    if (result.delta) {result.cosine=g;result.pdf=0.;return result;}
    let q=1.-g+2.*g*u;
    // Factored inverse CDF: stable at isotropy without a small-g approximation,
    // and at the forward/backward peaks without subtracting two squared terms.
    result.cosine=g>=0. ? 1.-2.*(1.-g)*(1.-g)*(1.-u)*(1.+g*u)/(q*q)
        : -1.+2.*(1.+g)*(1.+g)*u*(1.-g+g*u)/(q*q);
    result.cosine=clamp(result.cosine,-1.,1.);
    result.pdf=medium_phase(g,result.cosine);return result;
}

// Aggregate path coefficient accounting.
public struct MediumAggregate {
    public float3 extinction;
    public float3 emission;
    public float scattering_power;
};
public void medium_add(inout MediumAggregate sum,MediumSample m) {
    sum.extinction+=m.absorption+m.scattering;sum.emission+=m.emission;
    sum.scattering_power+=dot(m.scattering,REG_LUMINANCE);
}
