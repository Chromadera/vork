__exported import orthonormal_basis;
// Shared anisotropic GGX frame, response and sampling. All vectors point
// away from the interface. Zero anisotropy retains the existing isotropic path.
import vork_constants;
import openpbr_core;
import closure;
float3x3 microfacet_frame(Closure c) {
    let n=c.geometry_normal;
    var t=c.geometry_tangent-n*dot(n,c.geometry_tangent);
    if (dot(t,t)<1e-12) {t=cross(abs(n.y)<.99 ? float3(0.,1.,0.) : float3(1.,0.,0.),n);}
    t=normalize(t);
    return transpose(float3x3(t,cross(n,t),n));
}
// OpenPBR's constant RMS slope mapping, bounded away from a singular NDF.
float2 microfacet_alpha(Closure c) {
    let a=specular_alpha(c.specular_roughness);let an=c.specular_roughness_anisotropy;
    let ratio=1.-an;let ax=a*sqrt(2./(1.+ratio*ratio));
    return max(float2(ax,ratio*ax),float2(specular_alpha(0.)));
}
float microfacet_distribution(Closure c, float3 h) {
    if (c.specular_roughness_anisotropy<=0.) {return distribution(max(dot(c.geometry_normal,h),0.),specular_alpha(c.specular_roughness));}
    let local=mul(h,microfacet_frame(c));let a=microfacet_alpha(c);
    let q=float3(local.xy/a,local.z);let d=dot(q,q);
    return 1./(REG_PI*a.x*a.y*d*d);
}
float microfacet_visibility(Closure c, float3 view, float3 light) {
    let no=max(dot(c.geometry_normal,view),.0001);let ni=abs(dot(c.geometry_normal,light));
    if (c.specular_roughness_anisotropy<=0.) {return smith_visibility(no,ni,specular_alpha(c.specular_roughness),1e-20);}
    let frame_=microfacet_frame(c);let a=microfacet_alpha(c);
    let v=mul(view,frame_);let l=mul(light,frame_);
    let vo=length(float3(a*v.xy,no));let li=length(float3(a*l.xy,ni));
    return .5/max(ni*vo+no*li,1e-20);
}
float3 microfacet_sample(Closure c, float2 random) {
    let phi=REG_TAU*random.y;
    let slope=sqrt(random.x/max(1.-random.x,1e-20));let a=microfacet_alpha(c);
    return mul(normalize(float3(a*float2(cos(phi),sin(phi))*slope,1.)),transpose(microfacet_frame(c)));
}

// GGX visible-normal density and projected-disk sampler (Heitz, JCGT 2018).
// Keep the physical single-scattering response unchanged; only its proposal
// includes view-dependent masking, with matching reflection/refraction PDFs.
float transport_visible_scale(Closure c,float3 view) {
    let nv=dot(c.geometry_normal,view);
    if (c.specular_roughness_anisotropy>0.) {
        let local=mul(view,microfacet_frame(c));let alpha=microfacet_alpha(c);
        return 2./(nv+length(float3(alpha*local.xy,nv)));
    }
    let a=specular_alpha(c.specular_roughness);
    // G1(view)/cos(view), with the grazing cosine cancelled analytically.
    return 2./(nv+sqrt(a*a+(1.-a*a)*nv*nv));
}
float3 transport_visible_normal(Closure c,float3 view,float2 u) {
    let n=c.geometry_normal;
    let frame_=microfacet_frame(c);
    let x=c.specular_roughness_anisotropy>0. ? mul(float3(1.,0.,0.),transpose(frame_)) : basis_direction(n,float3(1.,0.,0.));
    let y=c.specular_roughness_anisotropy>0. ? mul(float3(0.,1.,0.),transpose(frame_)) : basis_direction(n,float3(0.,1.,0.));
    let a=c.specular_roughness_anisotropy>0. ? microfacet_alpha(c) : float2(specular_alpha(c.specular_roughness));
    let local=float3(dot(view,x),dot(view,y),dot(view,n));
    let sampled=ggx_visible_normal(local,a,u);
    return sampled.x*x+sampled.y*y+sampled.z*n;
}
