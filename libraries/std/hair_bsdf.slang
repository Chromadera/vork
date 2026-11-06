// Chiang et al. 2016, "A Practical and Controllable Hair and Fur Model".
// Independently implemented from the published equations; no Blender/Cycles code.
// This is the sole hair evaluator, shared by material programs and integrators.
import vork;
static const float HAIR_PI = 3.141592653589793;

// The paper's artist-friendly reflectance and pigment parameterizations.
public float3 hair_absorption(float3 color, float radial_roughness) {
    let b=clamp(radial_roughness,0.,1.);
    let d=5.969-.215*b+2.532*b*b-10.73*b*b*b+5.574*pow(b,4.)+.245*pow(b,5.);
    let x=log(clamp(color,float3(1e-6),float3(1.)))/d;
    return x*x;
}
public float3 hair_melanin(float melanin, float melanin_redness) {
    let concentration=-log(max(1.-clamp(melanin,0.,1.),1e-4));
    return concentration*lerp(float3(.506,.841,1.653),float3(.343,.733,1.924),clamp(melanin_redness,0.,1.));
}
public Hair principled_hair(float3 color, float roughness, float radial_roughness,
                            float coat, float ior, float offset) {
    Hair h;
    h.absorption_coefficient=hair_absorption(color,radial_roughness);
    h.color=max(color,float3(0.));
    h.roughness=roughness;h.radial_roughness=radial_roughness;
    h.coat=coat;h.ior=ior;h.offset=offset;
    return h;
}
float hair_fresnel(float cosine, float eta) {
    let c=clamp(abs(cosine),0.,1.);
    let sin_t2=(1.-c*c)/(eta*eta);
    if (sin_t2>=1.) return 1.;
    let ct=sqrt(max(0.,1.-sin_t2));
    let rs=(c-eta*ct)/max(c+eta*ct,1e-8);
    let rp=(eta*c-ct)/max(eta*c+ct,1e-8);
    return .5*(rs*rs+rp*rp);
}
// log I0: series at small x, asymptotic expansion at large x. Avoid exp overflow
// for smooth strands, whose longitudinal variance can approach zero.
float hair_log_i0(float x) {
    x=abs(x);
    if (x>12.) return x-.5*log(2.*HAIR_PI*x)+log(1.+1./(8.*x)+9./(128.*x*x));
    var term=1.;var sum=1.;
    for (int k=1;k<=24;k++) {term*=x*x/(4.*float(k*k));sum+=term;}
    return log(sum);
}
float hair_m(float si, float ci, float so, float co, float variance) {
    let v=max(variance,1e-5);
    let a=ci*co/v;let b=si*so/v;
    // 2 v sinh(1/v) = v exp(1/v) (1-exp(-2/v)).
    return exp(hair_log_i0(a)-b-1./v-log(v)-log(max(1.-exp(-2./v),1e-20)));
}
float hair_logistic_cdf(float x, float s) {return 1./(1.+exp(-x/s));}
float hair_n(float phi, float center, float s) {
    var x=phi-center;
    x-=2.*HAIR_PI*floor((x+HAIR_PI)/(2.*HAIR_PI));
    let e=exp(-abs(x)/s);
    return e/(s*(1.+e)*(1.+e)*(hair_logistic_cdf(HAIR_PI,s)-hair_logistic_cdf(-HAIR_PI,s)));
}
// Outgoing-dependent Chiang factors shared by evaluation and importance sampling.
struct HairDistribution {
    float3 t; float3 vo;
    float theta_o; float gamma_o; float gamma_t; float scale;
    float3 attenuation[4]; float shift[4]; float variance[4];
};
HairDistribution hair_distribution(Hair m,float3 tangent,float3 normal,float3 wo) {
    let t=normalize(tangent);
    let so=clamp(dot(wo,t),-1.,1.);
    let co=sqrt(max(1.-so*so,1e-8));
    let side=cross(t,wo);
    let vo=normalize(cross(dot(side,side)>0. ? side : cross(t,normal),t));
    let h=clamp(-dot(cross(t,vo),normal),-.99999,.99999);
    let gamma_o=asin(h);let eta=max(m.ior,1e-4);
    let eta_prime=sqrt(max(eta*eta-so*so,1e-8))/co;
    let gamma_t=asin(clamp(h/eta_prime,-1.,1.));
    let cos_theta_t=sqrt(max(1.-so*so/(eta*eta),1e-8));
    let transmittance=exp(-max(m.absorption_coefficient,float3(0.))*(2.*cos(gamma_t)/cos_theta_t));
    let f=hair_fresnel(co*cos(gamma_o),eta);
    HairDistribution d;
    d.attenuation[0]=float3(f);
    d.attenuation[1]=(1.-f)*(1.-f)*transmittance;
    d.attenuation[2]=d.attenuation[1]*transmittance*f;
    d.attenuation[3]=d.attenuation[2]*transmittance*f/max(float3(1.)-transmittance*f,float3(1e-6));
    let b=clamp(m.roughness,.001,1.);
    let v=pow(.726*b+.812*b*b+3.7*pow(b,20.),2.);
    let br=clamp(m.radial_roughness,.001,1.);
    let scale=max(sqrt(HAIR_PI/8.)*(.265*br+1.194*br*br+5.372*pow(br,22.)),1e-4);
    let theta_o=asin(so);let alpha=clamp(m.offset,-.5*HAIR_PI,.5*HAIR_PI);
    // Positive cuticle tilt follows Blender's root-to-tip curve convention.
    d.shift[0]=2.*alpha;d.shift[1]=-alpha;d.shift[2]=-4.*alpha;d.shift[3]=0.;
    let r=b*(1.-clamp(m.coat,0.,1.));
    let vr=pow(.726*r+.812*r*r+3.7*pow(r,20.),2.);
    d.variance[0]=vr;d.variance[1]=v*.25;d.variance[2]=v*4.;d.variance[3]=v*4.;
    d.t=t;d.vo=vo;d.theta_o=theta_o;d.gamma_o=gamma_o;d.gamma_t=gamma_t;d.scale=scale;return d;
}
float3 hair_response(HairDistribution d,float3 wi,out float pdf) {
    let si=clamp(dot(wi,d.t),-1.,1.);let ci=sqrt(max(1.-si*si,1e-8));
    let vi=(wi-d.t*si)/ci;
    let phi=atan2(dot(d.t,cross(d.vo,vi)),dot(d.vo,vi));
    var total=0.;float weights[4];
    for (int p=0;p<4;p++) {weights[p]=dot(d.attenuation[p],float3(.2126,.7152,.0722));total+=weights[p];}
    var result=float3(0.);pdf=0.;
    for (int p=0;p<4;p++) {
        let theta=d.theta_o+d.shift[p];
        let longitudinal=hair_m(si,ci,sin(theta),abs(cos(theta)),d.variance[p]);
        let azimuth=p==3 ? 1./(2.*HAIR_PI) : hair_n(phi,2.*float(p)*d.gamma_t-2.*d.gamma_o+float(p)*HAIR_PI,d.scale);
        let density=longitudinal*azimuth;
        result+=d.attenuation[p]*density;pdf+=weights[p]*density/max(total,1e-20);
    }
    return result;
}
// Cylindrical response includes the incident longitudinal cosine.
// pdf: hair_sample's solid-angle density of wi, for MIS.
public float3 hair_evaluate(Hair m,float3 tangent,float3 normal,float3 wo,float3 wi,out float pdf) {
    return hair_response(hair_distribution(m,tangent,normal,wo),wi,pdf);
}
public float3 hair_evaluate(Hair m,float3 tangent,float3 normal,float3 wo,float3 wi) {float pdf;return hair_evaluate(m,tangent,normal,wo,wi,pdf);}
// order: sampled scattering order p (0 = R reflection; >0 crosses the fibre).
public struct HairScatter { public float3 direction; public float3 response; public float pdf; public uint order; };
// Three independent dimensions: select/remap an attenuation lobe for azimuth,
// then invert its longitudinal spherical-Gaussian marginal and ring angle.
public HairScatter hair_sample(Hair h,float3 tangent,float3 normal,float3 wo,float3 u) {
    let d=hair_distribution(h,tangent,normal,wo);
    float weights[4];var total=0.;
    for (int i=0;i<4;i++) {weights[i]=dot(d.attenuation[i],float3(.2126,.7152,.0722));total+=weights[i];}
    var at=u.x*total;var p=3;
    for (int i=0;i<3;i++) {if(at<weights[i]) {p=i;break;}at-=weights[i];}
    let azimuth_u=clamp(at/max(weights[p],1e-20),0.,1.);
    let variance=max(d.variance[p],1e-5);
    let ct=clamp(1.+variance*log(max(u.y+(1.-u.y)*exp(-2./variance),1e-30)),-1.,1.);
    let st=sqrt(max(0.,1.-ct*ct));let theta=d.theta_o+d.shift[p];
    let si=clamp(-ct*sin(theta)+st*cos(2.*HAIR_PI*u.z)*abs(cos(theta)),-1.,1.);
    let ci=sqrt(max(0.,1.-si*si));
    var phi=2.*HAIR_PI*azimuth_u;
    if(p<3) {
        // Invert the lower half and reflect: avoid cancellation near CDF=1.
        let low=hair_logistic_cdf(-HAIR_PI,d.scale);
        let c=low+min(azimuth_u,1.-azimuth_u)*(1.-2.*low);
        let x=d.scale*(log(max(c,1e-30))-log(1.-c));
        phi=select(azimuth_u<=.5,x,-x)+2.*float(p)*d.gamma_t-2.*d.gamma_o+float(p)*HAIR_PI;
    }
    HairScatter sample;
    sample.direction=si*d.t+ci*(cos(phi)*d.vo+sin(phi)*cross(d.t,d.vo));
    sample.response=hair_response(d,sample.direction,sample.pdf);sample.order=uint(p);return sample;
}
// Finite emitters: deterministic area quadrature, same evaluator as punctual light.
public float3 hair_emitter(Hair h, float3 t, float3 n, float3 wo,
                          float3 center, float3 u, float3 v, bool ellipse, float radiance) {
    let area=length(cross(u,v))*(ellipse ? HAIR_PI : 4.);
    let facing=normalize(cross(u,v));var sum=float3(0.);
    for (int j=0;j<4;j++) for (int i=0;i<4;i++) {
        let a=(float(i)+.5)/4.;let b=(float(j)+.5)/4.;
        var xy=float2(2.*a-1.,2.*b-1.);
        if (ellipse) xy=sqrt(a)*float2(cos(2.*HAIR_PI*b),sin(2.*HAIR_PI*b));
        let d=center+u*xy.x+v*xy.y;let r2=max(dot(d,d),1e-10);let wi=d*rsqrt(r2);
        sum+=hair_evaluate(h,t,n,wo,wi)*abs(dot(facing,wi))/r2;
    }
    return sum*(radiance*area/16.);
}
public float3 hair_uniform_world(Hair m, float3 tangent, float3 normal, float3 wo) {
    // Each longitudinal density integrates to one over sin(theta_i), and each
    // azimuth density to one over [-pi,pi]. Integrate analytically, even at
    // arbitrarily small roughness; fixed angular grids miss narrow lobes.
    let t=normalize(tangent);let so=clamp(dot(wo,t),-1.,1.);
    let co=sqrt(max(1.-so*so,1e-8));let vo=(wo-t*so)/co;
    let h=clamp(-dot(cross(t,vo),normal),-.99999,.99999);
    let eta=max(m.ior,1e-4);
    let eta_prime=sqrt(max(eta*eta-so*so,1e-8))/co;
    let gamma_t=asin(clamp(h/eta_prime,-1.,1.));
    let cos_theta_t=sqrt(max(1.-so*so/(eta*eta),1e-8));
    let tr=exp(-max(m.absorption_coefficient,float3(0.))*(2.*cos(gamma_t)/cos_theta_t));
    let f=hair_fresnel(co*sqrt(1.-h*h),eta);
    let tt=(1.-f)*(1.-f)*tr;
    return float3(f)+tt/max(float3(1.)-tr*f,float3(1e-6));
}

public Hair principled_hair_absorption(float3 absorption_coefficient, float roughness,
    float radial_roughness, float coat, float ior, float offset) {
    Hair h;h.absorption_coefficient=max(absorption_coefficient,float3(0.));
    h.roughness=roughness;h.radial_roughness=radial_roughness;
    h.coat=coat;h.ior=ior;h.offset=offset;return h;
}
public Hair principled_hair_melanin(float melanin, float melanin_redness, float3 tint,
    float roughness, float radial_roughness, float coat, float ior, float offset,
    float random_color, float random_roughness, float random) {
    let variation=2.*clamp(random,0.,1.)-1.;
    let amount=clamp(melanin*(1.+variation*clamp(random_color,0.,1.)),0.,1.);
    let factor=1.+variation*clamp(random_roughness,0.,1.);
    let radial=clamp(radial_roughness*factor,0.,1.);
    return principled_hair_absorption(hair_melanin(amount,melanin_redness)+hair_absorption(tint,radial),
        clamp(roughness*factor,0.,1.),radial,coat,ior,offset);
}

// Fibre-bundle transport moments, from Zinke et al. (SIGGRAPH 2008),
// https://www.cemyuksel.com/research/dualscattering/dualscattering.pdf .
// These are integrals of the same Chiang closure, not a second hair model.
// Geometry supplies finite crossing counts; no fitted bundle-density constant.
public struct HairTransportMoments {
    public float3 forward;
    public float3 backward;
    public float3 forward_shift;
    public float3 backward_shift;
    public float3 forward_variance;
    public float3 backward_variance;
};
// Integral of the periodic trimmed logistic over an interval of length <=2pi.
float hair_n_integral(float low, float width, float center, float scale) {
    var a=low-center;
    a-=2.*HAIR_PI*floor((a+HAIR_PI)/(2.*HAIR_PI));
    let b=a+width;
    let normalization=hair_logistic_cdf(HAIR_PI,scale)-hair_logistic_cdf(-HAIR_PI,scale);
    var integral=hair_logistic_cdf(min(b,HAIR_PI),scale)-hair_logistic_cdf(a,scale);
    if (b>HAIR_PI) {integral+=hair_logistic_cdf(b-2.*HAIR_PI,scale)-hair_logistic_cdf(-HAIR_PI,scale);}
    return integral/normalization;
}
// Through-fibre transmission at a resolved impact parameter. The R lobe never
// enters the fibre: its reflected energy belongs to scattered connections, not
// transmission along the current visibility ray. Retain the transmitted orders.
public float3 hair_forward_at(Hair m,float3 tangent,float3 normal,float3 outgoing) {
    let d=hair_distribution(m,tangent,normal,outgoing);
    var forward=float3(0.);
    for (uint p=1u;p<4u;p++) {
        let fraction=p==3u ? .5 : hair_n_integral(.5*HAIR_PI,HAIR_PI,2.*float(p)*d.gamma_t-2.*d.gamma_o+float(p)*HAIR_PI,d.scale);
        forward+=d.attenuation[p]*fraction;
    }
    return forward;
}
public HairTransportMoments hair_transport_moments(Hair m, float sine_theta) {
    let so=clamp(sine_theta,-.99999,.99999);let co=sqrt(1.-so*so);
    let eta=max(m.ior,1e-4);let eta_prime=sqrt(max(eta*eta-so*so,1e-8))/co;
    let cos_theta_t=sqrt(max(1.-so*so/(eta*eta),1e-8));
    let b=clamp(m.roughness,.001,1.);
    let variance=pow(.726*b+.812*b*b+3.7*pow(b,20.),2.);
    let r=b*(1.-clamp(m.coat,0.,1.));
    let reflection_variance=pow(.726*r+.812*r*r+3.7*pow(r,20.),2.);
    let br=clamp(m.radial_roughness,.001,1.);
    let scale=max(sqrt(HAIR_PI/8.)*(.265*br+1.194*br*br+5.372*pow(br,22.)),1e-4);
    let alpha=clamp(m.offset,-.5*HAIR_PI,.5*HAIR_PI);
    float shifts[4]={-2.*alpha,alpha,4.*alpha,0.};
    float variances[4]={reflection_variance,variance*.25,variance*4.,variance*4.};
    HairTransportMoments result={};
    // Uniform projected cross-section measure dh/2. Azimuth integrates exactly;
    // midpoint quadrature only integrates smooth Fresnel/absorption across h.
    for (uint j=0u;j<8u;j++) {
        let h=2.*(float(j)+.5)/8.-1.;let go=asin(h);
        let gt=asin(clamp(h/eta_prime,-1.,1.));
        let tr=exp(-max(m.absorption_coefficient,float3(0.))*(2.*cos(gt)/cos_theta_t));
        let f=hair_fresnel(co*cos(go),eta);
        float3 a[4];a[0]=float3(f);a[1]=(1.-f)*(1.-f)*tr;
        a[2]=a[1]*tr*f;a[3]=a[2]*tr*f/max(float3(1.)-tr*f,float3(1e-6));
        for (uint p=0u;p<4u;p++) {
            let front=p==3u ? .5 : hair_n_integral(.5*HAIR_PI,HAIR_PI,2.*float(p)*gt-2.*go+float(p)*HAIR_PI,scale);
            let af=a[p]*(front/8.);let ab=a[p]*((1.-front)/8.);
            result.forward+=af;result.backward+=ab;
            result.forward_shift+=af*shifts[p];result.backward_shift+=ab*shifts[p];
            result.forward_variance+=af*variances[p];result.backward_variance+=ab*variances[p];
        }
    }
    let af=max(result.forward,float3(1e-8));let ab=max(result.backward,float3(1e-8));
    result.forward_shift/=af;result.backward_shift/=ab;
    result.forward_variance/=af;result.backward_variance/=ab;
    return result;
}
// Local backscattering for a finite bundle. Unlike the infinite-depth paper
// sums, finite sums vanish for an isolated fibre (neighbours ==0).
public float3 hair_bundle_backscatter(HairTransportMoments m, float neighbours,
                                     float sine_i, float sine_o, float phi, float3 extra_shift, float3 extra_variance, bool azimuth_spread) {
    if (neighbours<=0. || (!azimuth_spread && cos(phi)<=0.)) {return float3(0.);}
    let count=max(neighbours,0.);let r=clamp(m.forward*m.forward,float3(0.),float3(1.));
    let d=max(float3(1.)-r,float3(1e-8));let q=pow(r,float3(count));
    let series=(float3(1.)-q)/d;
    let first=m.backward*r*series;
    // Sum over i=1..n, j=0..i-1, k=j+1..n of r^(i-j-1+k).
    let numerator=r*series*(float3(1.)+q*r)-2.*count*q*r;
    let third=pow(m.backward, float3(3.))*max(numerator/(d*d),float3(0.));
    let attenuation=first+third;
    let mean_first=max(float3(1.)/d-count*q/max(float3(1.)-q,float3(1e-8)),float3(1.));
    let series_derivative=(float3(1.)-q-count*pow(max(r,float3(1e-8)),float3(count-1.))*d)/(d*d);
    let derivative=(series+r*series_derivative)*(float3(1.)+q*r)
        +r*series*(count+1.)*q-2.*count*(count+1.)*q;
    let mean_third=max(r*(derivative/max(numerator,float3(1e-8))+2./d),float3(1.));
    let total=max(attenuation,float3(1e-8));
    let shift=(first*(2.*mean_first*m.forward_shift+m.backward_shift)
              +third*(2.*mean_third*m.forward_shift+3.*m.backward_shift))/total;
    let variance=max((first*(2.*mean_first*m.forward_variance+m.backward_variance)
                     +third*(2.*mean_third*m.forward_variance+3.*m.backward_variance))/total,float3(1e-5));
    let theta_i=asin(clamp(sine_i,-1.,1.));let theta_o=asin(clamp(sine_o,-1.,1.));
    let delta=float3(theta_i+theta_o)-shift-extra_shift;
    let broadened=variance+extra_variance;
    let gaussian=exp(-delta*delta/(2.*broadened))/sqrt(2.*HAIR_PI*broadened);
    let cosine=cos(.5*(theta_o-theta_i));
    return (azimuth_spread ? max(1.-abs(phi)/HAIR_PI,0.) : 1.)*2.*attenuation*gaussian*sqrt(max(1.-sine_i*sine_i,0.))/(HAIR_PI*max(cosine*cosine,1e-8));
}

