// Scene-linear colour utilities, independently implemented from Blender's manual
// and standard RGB/HSV/HSL definitions. No Blender source is incorporated.
public float3 color_from_hsv(float3 hsv) {
    float3 rgb=saturate(abs(frac(hsv.x+float3(0.,2./3.,1./3.))*6.-3.)-1.);
    return hsv.z*lerp(float3(1.),rgb,hsv.y);
}
public float3 color_to_hsv(float3 rgb) {
    float hi=max(rgb.x,max(rgb.y,rgb.z)), lo=min(rgb.x,min(rgb.y,rgb.z)), delta=hi-lo;
    float h=0.;
    if(delta>0.) {
        if(hi==rgb.x) h=(rgb.y-rgb.z)/delta;
        else if(hi==rgb.y) h=2.+(rgb.z-rgb.x)/delta;
        else h=4.+(rgb.x-rgb.y)/delta;
        h=frac(h/6.);
    }
    return float3(h,hi==0.?0.:delta/hi,hi);
}
float3 color_to_hsl(float3 rgb) {
    float3 hsv=color_to_hsv(rgb);
    float lo=min(rgb.x,min(rgb.y,rgb.z)), l=(hsv.z+lo)*0.5;
    float denominator=1.-abs(2.*l-1.);
    return float3(hsv.x,denominator==0.?0.:(hsv.z-lo)/denominator,l);
}
float3 color_from_hsl(float3 hsl) {
    float c=(1.-abs(2.*hsl.z-1.))*hsl.y;
    return color_from_hsv(float3(hsl.x,1.,1.))*c+(hsl.z-c*0.5);
}
public enum RampInterpolation { Constant, Linear, Ease, Cardinal, B_Spline };
public enum RampColorMode { RGB, HSV, HSL };
public enum HueInterpolation { Near, Far, CW, CCW };
public struct RampStop { public float position; public float4 color; };
float ramp_hue(float a,float b,HueInterpolation mode) {
    float d=b-a;
    if(mode==HueInterpolation.Near) { if(d>0.5) b-=1.; if(d< -0.5) b+=1.; }
    if(mode==HueInterpolation.Far) { if(d>0. && d<0.5) b-=1.; if(d<=0. && d> -0.5) b+=1.; }
    if(mode==HueInterpolation.CW && d>0.) b-=1.;
    if(mode==HueInterpolation.CCW && d<0.) b+=1.;
    return b;
}
// Stops are ordered by position. Empty ramps return transparent black.
public float4 color_ramp<let N:int>(float factor,RampStop stops[N],
    RampInterpolation interpolation=RampInterpolation.Linear,RampColorMode mode=RampColorMode.RGB,
    HueInterpolation hue=HueInterpolation.Near) {
    if(N==0) return float4(0.);
    if(N==1 || factor<stops[0].position) return stops[0].color;
    int i=0;
    for(int j=1;j<N;j++) if(factor>=stops[j].position) i=j;
    if(i==N-1 || interpolation==RampInterpolation.Constant) return stops[i].color;
    float width=stops[i+1].position-stops[i].position;
    float t=width>0.?(factor-stops[i].position)/width:0.;
    if(interpolation==RampInterpolation.Ease) t=t*t*(3.-2.*t);
    float4 a=stops[i].color,b=stops[i+1].color;
    if(mode!=RampColorMode.RGB) {
        a.xyz=mode==RampColorMode.HSV?color_to_hsv(a.xyz):color_to_hsl(a.xyz);
        b.xyz=mode==RampColorMode.HSV?color_to_hsv(b.xyz):color_to_hsl(b.xyz);
        b.x=ramp_hue(a.x,b.x,hue);
        float4 result=lerp(a,b,t);
        result.xyz=mode==RampColorMode.HSV?color_from_hsv(result.xyz):color_from_hsl(result.xyz);
        return result;
    }
    if(interpolation==RampInterpolation.Cardinal || interpolation==RampInterpolation.B_Spline) {
        float4 p=stops[max(i-1,0)].color,q=stops[min(i+2,N-1)].color;
        float t2=t*t,t3=t2*t;
        if(interpolation==RampInterpolation.B_Spline)
            return ((1.-3.*t+3.*t2-t3)*p+(4.-6.*t2+3.*t3)*a+(1.+3.*t+3.*t2-3.*t3)*b+t3*q)/6.;
        // Cardinal tension 0.71, as specified by the Blender ramp.
        float4 m0=0.71*(b-p),m1=0.71*(q-a);
        return (2.*t3-3.*t2+1.)*a+(t3-2.*t2+t)*m0+(-2.*t3+3.*t2)*b+(t3-t2)*m1;
    }
    return lerp(a,b,t);
}
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
