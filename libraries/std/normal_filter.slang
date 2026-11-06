// Spherical normal moments and GGX roughness filtering. Shared by table and program materials.
import vork_openpbr_constants;

public float3 normal_map_unit(float3 n) {let squared=dot(n,n);return squared>0. ? n*rsqrt(squared) : float3(0.,0.,1.);}
public float3 filtered_normal(float3 n0,float3 n1,float3 n2,float3 n3,out float2 variance) {
    let mean=((n0+n1)+(n2+n3))*.25;
    let d0=n0-mean;let d1=n1-mean;let d2=n2-mean;let d3=n3-mean;
    // E|N-mean|^2 = 1-|mean|^2 for unit normals. Differences make a
    // constant map's variance exactly zero despite encoding/FP error.
    let total_variance=(dot(d0,d0)+dot(d1,d1)+dot(d2,d2)+dot(d3,d3))*.25;
    // The tangential covariance supplies both principal widths. Fit each
    // axis before reducing to an area-preserving isotropic GGX lobe.
    variance=float2(total_variance);
    let mean_squared=dot(mean,mean);
    if (mean_squared>0.) {
        let direction=mean*rsqrt(mean_squared);
        let seed=select(abs(direction.x)<abs(direction.z),float3(1.,0.,0.),float3(0.,0.,1.));
        let axis0=normalize(cross(direction,seed));let axis1=cross(direction,axis0);
        let q0=float2(dot(d0,axis0),dot(d0,axis1));let q1=float2(dot(d1,axis0),dot(d1,axis1));
        let q2=float2(dot(d2,axis0),dot(d2,axis1));let q3=float2(dot(d3,axis0),dot(d3,axis1));
        let diagonal=(q0*q0+q1*q1+q2*q2+q3*q3)*.25;
        let off=(q0.x*q0.y+q1.x*q1.y+q2.x*q2.y+q3.x*q3.y)*.25;
        let difference=diagonal.x-diagonal.y;
        let gap=sqrt(difference*difference+4.*off*off);
        variance=max(float2(total_variance-gap,total_variance+gap),float2(0.));
    }
    // A cancelled mean has no axis and retains total dispersion above.
    return mean;
}

// Mean cosine of GGX's normalized projected NDF, with x=alpha^2.
// This bounded first spherical moment exists despite infinite slope variance.
float ggx_normal_length(float x) {
    if (x<=0.) {return 1.;}
    let b=1.-x;
    if (b<REG_GGX_MOMENT_SERIES_LIMIT) {
        // Eight terms have <1e-9 truncation error for b<1/8.
        return 2./3.+b*(2./15.+b*(2./35.+b*(2./63.+b*(2./99.+b*(2./143.+b*(2./195.+b*(2./255.)))))));
    }
    let t=sqrt(b);
    // atanh(t)=log((1+t)/sqrt(x)), also finite when 1-x rounds to 1.
    return (1.-x*log((1.+t)/sqrt(x))/t)/b;
}
float ggx_inverse_normal_moment(float x,float moment,float target) {
    if (target>=moment) {return x;}
    let deficit=1.-target;
    let z=2.*(log(deficit)-REG_GGX_NORMAL_LOG_MIN)/REG_GGX_NORMAL_LOG_RANGE-1.;
    var b1=0.;var b2=0.;
    [unroll] for (int i=REG_GGX_NORMAL_INVERSE_DEGREE;i>0;i--) {
        let b=2.*z*b1-b2+vork_ggx_normal_inverse(i);b2=b1;b1=b;
    }
    return clamp(deficit/(z*b1-b2+vork_ggx_normal_inverse(0)),x,1.);
}
public float filtered_ggx_roughness(float roughness,float2 normal_variance) {
    let r2=roughness*roughness;let x=max(r2*r2,REG_GGX_ALPHA_MIN*REG_GGX_ALPHA_MIN);
    let moment=ggx_normal_length(x);
    let target=max(float2(2./3.),moment*sqrt(max(float2(1.)-normal_variance,float2(0.))));
    if (all(target>=float2(moment))) {return roughness;}
    // The inverse returns alpha^2, already respecting the BSDF floor. The
    // geometric mean of principal alphas preserves the anisotropic peak area.
    let u=ggx_inverse_normal_moment(x,moment,target.x);
    let v=ggx_inverse_normal_moment(x,moment,target.y);
    return sqrt(sqrt(sqrt(u*v)));
}
