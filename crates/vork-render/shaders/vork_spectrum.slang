// CIE approximation: Wyman, Sloan and Shirley, JCGT 2(2), 2013, eq. 4.
// https://jcgt.org/published/0002/02/01/ (formula implementation, no bundled code).
import vork_constants;
float spectral_gaussian(float nm,float center,float left,float right) {let t=(nm-center)*(nm<center ? left : right);return exp(-.5*t*t);}
float3 spectral_xyz(float nm) {
    return float3(1.056*spectral_gaussian(nm,599.8,.0264,.0323)+.362*spectral_gaussian(nm,442.,.0624,.0374)-.065*spectral_gaussian(nm,501.1,.049,.0382),
    .821*spectral_gaussian(nm,568.8,.0213,.0247)+.286*spectral_gaussian(nm,530.9,.0613,.0322),
    1.217*spectral_gaussian(nm,437.,.0845,.0278)+.681*spectral_gaussian(nm,459.,.0385,.0725));
}
public float3 spectral_response(float nm) {
    let xyz=spectral_xyz(nm);
    // Analytic CIE 1931 fit, linear Rec.709. Never clamp negative matching lobes.
    return float3(dot(xyz,float3(3.2406,-1.5372,-.4986)),dot(xyz,float3(-.9689,1.8758,.0415)),dot(xyz,float3(.0557,-.204,1.057)))*REG_SPECTRAL_SPAN_NM/106.85/float3(1.03996270,.98474418,1.03404348);
}
public float3 spectral_value(float3 rgb,float nm,bool emission) {
    // Extrapolate the RGB basis outside visible wavelengths for authored wavelength mappings.
    // This is an explicit synthetic spectrum, not inferred UV/IR material data.
    if(nm<=0.) return float3(0.);
    // RGB primary partition, white is flat. Positive and bounded for reflectance.
    // Smooth transitions, not an emission-only RGB colour filter.
    let r=smoothstep(550.,620.,nm);let b=1.-smoothstep(450.,510.,nm);let g=1.-r-b;
    let illuminant=emission ? 2.82020563/(pow(nm*.001,5.)*(exp(14387.76877/(nm*.001*6504.))-1.)) : 1.;
    return float3(dot(max(rgb,float3(0.)),float3(r,g,b))*illuminant);
}
// OpenPBR 1.1: Cauchy dispersion at the Fraunhofer d/F/C lines, nm.
public float spectral_ior(float n,float nm,float scale,float abbe) {
    if(nm<=0. || scale<=0.) return n;
    let b=(n-1.)*scale/abbe/(1./(486.13*486.13)-1./(656.27*656.27));
    return n+b*(1./(nm*nm)-1./(587.56*587.56));
}
