// Analytic CIE 1931 matching-function fit (Wyman, Sloan, Shirley 2013).
// Integrate Planck's law for blackbody; wavelength takes nanometres.
float color_gaussian(float wavelength,float center,float left,float right) {
    float t=(wavelength-center)*(wavelength<center?left:right);return exp(-0.5*t*t);
}
float3 color_xyz(float nm) {
    return float3(1.056*color_gaussian(nm,599.8,0.0264,0.0323)+0.362*color_gaussian(nm,442.,0.0624,0.0374)-0.065*color_gaussian(nm,501.1,0.049,0.0382),
        0.821*color_gaussian(nm,568.8,0.0213,0.0247)+0.286*color_gaussian(nm,530.9,0.0613,0.0322),
        1.217*color_gaussian(nm,437.,0.0845,0.0278)+0.681*color_gaussian(nm,459.,0.0385,0.0725));
}
float3 color_xyz_rgb(float3 xyz) {
    return float3(dot(xyz,float3(3.2406,-1.5372,-0.4986)),dot(xyz,float3(-0.9689,1.8758,0.0415)),dot(xyz,float3(0.0557,-0.204,1.057)));
}
public float3 wavelength(float nm=500.) {
    if(nm<380. || nm>780.) return float3(0.);
    return max(color_xyz_rgb(color_xyz(nm))*0.4,0.);
}
public float3 blackbody(float temperature=1500.) {
    float3 xyz=0.; float t=max(temperature,1.);
    for(int i=0;i<81;i++) {
        float nm=380.+5.*i, micrometers=nm*0.001;
        float power=1./(pow(micrometers,5.)*(exp(min(14387.76877/(micrometers*t),80.))-1.));
        xyz+=color_xyz(nm)*power*((i==0 || i==80)?0.5:1.);
    }
    return xyz.y>0.?max(color_xyz_rgb(xyz/xyz.y),0.):float3(0.);
}
