// Belcour & Barla 2017, spectral thin-film interference: Fourier-domain CIE
// sensitivity fits and the Airy multiple-reflection series. Thickness is µm,
// matching OpenPBR. Fresnel amplitudes retain s/p polarization and phase signs.
import vork_constants;
import openpbr_core;
import closure;
float3 film_xyz_to_rgb(float3 xyz) {
    return float3(dot(xyz,float3(3.2404542,-1.5371385,-.4985314)),
                  dot(xyz,float3(-.969266,1.8760108,.041556)),
                  dot(xyz,float3(.0556434,-.2040259,1.0572252)));
}
float3 film_sensitivity(float opd, float phase) {
    let p=REG_TAU*opd*1e-9;
    let variance=float3(4.3278e9,9.3046e9,6.6121e9);
    let dc=float3(5.4856e-13,4.4201e-13,5.2481e-13)*sqrt(REG_TAU*variance);
    let extra=9.7470e-14*sqrt(REG_TAU*4.5282e9);
    var xyz=dc*cos(float3(1.6810e6,1.7953e6,2.2084e6)*p+phase)*exp(-p*p*variance);
    xyz.x+=extra*cos(2.2399e6*p+phase)*exp(-4.5282e9*p*p);
    // Normalize the fitted observer's equal-energy white to scene-linear RGB
    // white. Otherwise the zero-optical-path limit acquires a spurious tint.
    return film_xyz_to_rgb(xyz)/film_xyz_to_rgb(dc+float3(extra,0.,0.));
}
float3 film_polarized(float r12, float r23, float opd) {
    let R12=r12*r12;let R23=r23*r23;let product=min(R12*R23,.999999);
    let transmitted=1.-R12;let series=transmitted*transmitted*R23/(1.-product);
    var result=float3(R12+series);
    var coefficient=series-transmitted;
    // The round-trip amplitude has the sign of the two internal reflections.
    let phase=(-r12*r23)<0. ? REG_PI : 0.;
    for (uint m=1u;m<=3u;m++) {
        coefficient*=sqrt(product);
        result+=2.*coefficient*film_sensitivity(float(m)*opd,float(m)*phase);
    }
    return max(result,float3(0.));
}
float3 film_dielectric(float cosine, float eta, float film_ior, float thickness) {
    let c=clamp(cosine,0.,1.);let s2=max(1.-c*c,0.);
    let cf2=1.-s2/(film_ior*film_ior);let cb2=1.-s2/(eta*eta);
    if (cf2<=0. || cb2<=0.) {return float3(1.);}
    let cf=sqrt(cf2);let cb=sqrt(cb2);
    let rs12=(c-film_ior*cf)/max(c+film_ior*cf,1e-20);
    let rp12=(film_ior*c-cf)/max(film_ior*c+cf,1e-20);
    let rs23=(film_ior*cf-eta*cb)/(film_ior*cf+eta*cb);
    let rp23=(eta*cf-film_ior*cb)/(eta*cf+film_ior*cb);
    let opd=2.*film_ior*thickness*1000.*cf;
    return .5*(film_polarized(rs12,rs23,opd)+film_polarized(rp12,rp23,opd));
}
float3 film_reflection(Closure c, float cosine, float3 original) {
    if (c.thin_film_weight<=0. || c.thin_film_thickness<=0.) {return original;}
    let dielectric=film_dielectric(cosine,c.specular_ior,c.thin_film_ior,c.thin_film_thickness)*c.specular_color;
    // OpenPBR F82 supplies RGB reflectance rather than complex optical constants.
    // Its effective RGB dielectric IOR retains the authored normal reflectance.
    let root=sqrt(clamp(c.base_weight*c.base_color,float3(0.),float3(.9999)));
    let eta=(1.+root)/(1.-root);
    var metal=float3(0.);
    if (c.base_metalness>0.) {
        for (uint i=0u;i<3u;i++) {metal[i]=film_dielectric(cosine,eta[i],c.thin_film_ior,c.thin_film_thickness)[i];}
        let base=c.base_weight*c.base_color;let mu=1.-cosine;
        let schlick=base+(1.-base)*pow(mu,5.);
        let tinted=max(schlick-metal_edge_correction(base,c.specular_color)*cosine*pow(mu,6.),float3(0.));
        metal*=tinted/max(schlick,float3(1e-20));
    }
    return lerp(original,c.specular_weight*lerp(dielectric,metal,c.base_metalness),c.thin_film_weight);
}

// Coincident thin interfaces sum their internal reflection series. Keep this
// response shared by direct lighting, traced reflections and path evaluation.
float3 surface_reflection(Closure c, float cosine) {
    let original=reflection_fresnel(c.base_color,c.base_weight,c.base_metalness,c.specular_weight,c.specular_color,cosine,c.specular_ior);
    var reflected=film_reflection(c,cosine,original);
    if(c.geometry_thin_walled && c.transmission_weight>0.) {
        var f=float3(fresnel(cosine,c.specular_ior));
        if(c.thin_film_weight>0. && c.thin_film_thickness>0.) {
            f=lerp(f,film_dielectric(cosine,c.specular_ior,c.thin_film_ior,c.thin_film_thickness),c.thin_film_weight);
        }
        reflected+=c.specular_weight*c.specular_color*c.transmission_weight*(1.-c.base_metalness)*(2.*f/(1.+f)-f);
    }
    return reflected;
}
