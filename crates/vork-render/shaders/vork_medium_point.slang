// Point coefficients and segment numerics shared by raster quadrature and path
// transport. No lighting, camera, froxel, shadow, or attachment interface.
import vork_constants;
__exported import vork_medium_types;
public extern uint medium_word(uint index);
public struct MediumTransportContext {
    public uint input_base;
    public float4x4 inverse;
    public float sample_fraction;
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
// Draw from the normalized extinction measure on a finite homogeneous cell.
// Sampling lighting uniformly and multiplying its mean by the extinction
// integral would retain a cell-size-dependent bias for varying illumination.
public float medium_extinction_distance(float sigma,float distance,float u) {
    if(u<=0.) {return 0.;}
    if(u>=1.) {return distance;}
    let tau=sigma*distance;
    if(tau==0.) {return u*distance;}
    if(tau<=REG_MEDIUM_INTEGRAL_SERIES_LIMIT) {
        let integral=1.+tau*(-.5+tau*(1./6.+tau*(-1./24.+tau*(1./120.-tau/720.))));
        let x=u*tau*integral;
        // -log(1-x)/x; cancel tau algebraically so tiny optical depths
        // retain their uniform limit without dividing subnormal quantities.
        let log_ratio=1.+x*(.5+x*(1./3.+x*(.25+x*(.2+x*(1./6.+x*(1./7.+x*.125))))));
        return distance*(u*integral*log_ratio);
    }
    let mass=1.-exp(-min(tau,REG_MEDIUM_EXP_LIMIT));
    return -log(1.-u*mass)/sigma;
}

// Extinction stays normalized until optical depth is formed. Individually finite
// overlapping coefficients may have an unrepresentable sum, while source ratios
// and the integrated radiance remain finite. Path's coefficient API is unchanged.
public struct MediumExtinction {
    public float3 value;
    public float3 scale;
};
public void medium_extinction_add(inout MediumExtinction sum,MediumSample m) {
    let scale=max(sum.scale,max(m.absorption,m.scattering));
    let divisor=select(scale>float3(0.),scale,float3(1.));
    sum.value=sum.value*(medium_ratio(sum.scale,divisor))+medium_ratio(m.absorption,divisor)+medium_ratio(m.scattering,divisor);
    sum.scale=scale;
}
public float3 medium_optical_depth(MediumExtinction sum,float distance) {
    let sigma=sum.scale*sum.value;
    // Preserve tiny representable optical depths when scale*distance alone
    // would underflow. Only an overflowing coefficient sum needs reordered math.
    return select(isfinite(sigma),sigma*distance,(sum.scale*distance)*sum.value);
}
public bool medium_extinction_equal(MediumExtinction sum,uint a,uint b) {
    let sigma=sum.scale*sum.value;
    return (isfinite(sigma[a]) && sigma[a]==sigma[b])
        || (sum.scale[a]==sum.scale[b] && sum.value[a]==sum.value[b]);
}
// Invert upstream's extinction measure in scaled distance units only when the
// coefficient sum overflows. Reuse its small-optical-depth series unchanged.
public float medium_extinction_distance(MediumExtinction sum,uint channel,float distance,float u) {
    if(u<=0.) {return 0.;}
    if(u>=1.) {return distance;}
    let sigma=sum.scale[channel]*sum.value[channel];
    if(isfinite(sigma)) {return medium_extinction_distance(sigma,distance,u);}
    return medium_ratio(medium_extinction_distance(sum.value[channel],sum.scale[channel]*distance,u),sum.scale[channel]);
}
// Equilibrium source per unit extinction; the same ratio closes infinite tails.
public float3 medium_equilibrium(float3 coefficient,MediumExtinction sum) {
    let scale=select(sum.scale>float3(0.),sum.scale,float3(1.));
    let sigma=select(sum.value>float3(0.),sum.value,float3(1.));
    let raw=sum.scale*sum.value;
    let denominator=select(raw>float3(0.),raw,float3(1.));
    return select(isfinite(raw),medium_ratio(coefficient,denominator),(medium_ratio(coefficient,scale))/sigma);
}
public float medium_optical_distance(MediumExtinction sum,uint channel,float optical) {
    let sigma=sum.scale[channel]*sum.value[channel];
    return isfinite(sigma) ? medium_ratio(optical,sigma)
        : medium_ratio(optical/sum.value[channel],sum.scale[channel]);
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

// Cancellation-free optical mass and inverse CDF near vacuum. The existing
// integral-series boundary keeps the omitted terms below FP32 precision.
public float medium_optical_mass(float tau) {
    if(tau<=REG_MEDIUM_INTEGRAL_SERIES_LIMIT) {
        return tau*(1.+tau*(-.5+tau*(1./6.+tau*(-1./24.+tau*(1./120.-tau/720.)))));
    }
    return 1.-exp(-tau);
}
public float medium_optical_quantile(float mass) {
    if(mass<=REG_MEDIUM_INTEGRAL_SERIES_LIMIT) {
        return mass*(1.+mass*(.5+mass*(1./3.+mass*(.25+mass*(.2+mass*(1./6.+mass*(1./7.+mass/8.)))))));
    }
    return -log(1.-mass);
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

// Aggregate coefficient accounting and front-to-back source composition.
public struct MediumAggregate {
    public float3 extinction;
    public float3 emission;
    public float scattering_power;
};
public void medium_add(inout MediumAggregate sum,MediumSample m) {
    sum.extinction+=m.absorption+m.scattering;sum.emission+=m.emission;
    sum.scattering_power+=dot(m.scattering,REG_LUMINANCE);
}
public void medium_append(inout MediumTransport prefix,MediumTransport suffix) {
    prefix.source+=medium_received_source(prefix.transmission,suffix.source);prefix.transmission*=suffix.transmission;prefix.optical_depth+=suffix.optical_depth;
}

// Balance weights for a uniform mixture of the three normalized extinction
// measures. Log densities avoid overflow for opaque or spectrally disparate
// cells; every channel has support and every weight is bounded by three.
public float3 medium_spectral_weights(float3 log_pdf) {
    let peak=max(log_pdf.x,max(log_pdf.y,log_pdf.z));
    let density=exp(log_pdf-peak);
    return 3.*density/(density.x+density.y+density.z);
}
public float3 medium_extinction_sample(MediumExtinction sum,float distance,float u,out float offset) {
    let draw=min(u*3.,2.9999998);let channel=uint(draw);
    offset=medium_extinction_distance(sum,channel,distance,frac(draw));
    let tau=medium_optical_depth(sum,distance);
    let local=medium_optical_depth(sum,offset);
    // The unit-interval PDF tends to one as optical depth tends to zero.
    let small=min(tau,float3(REG_MEDIUM_INTEGRAL_SERIES_LIMIT));
    let series=1.+small*(-.5+small*(1./6.+small*(-1./24.+small*(1./120.-small/720.))));
    let mass=float3(1.)-exp(-tau);
    let log_mass=log(select(mass>float3(0.),mass,float3(1.)));
    let log_tau=log(sum.scale)+log(sum.value)+log(distance);
    let log_pdf=select(tau<=float3(REG_MEDIUM_INTEGRAL_SERIES_LIMIT),-log(series),log_tau-log_mass)-local;
    return medium_spectral_weights(log_pdf);
}
public float3 medium_log_extinction(MediumExtinction sum) {return log(sum.scale)+log(sum.value);}
public float3 medium_relative_optical(float3 log_sigma,uint channel,float optical) {
    return optical==0. ? float3(0.) : exp(log_sigma-log_sigma[channel])*optical;
}
