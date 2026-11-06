import vork;
// Original RGB single-scattering implementation of the Nishita atmospheric
// transport model. Blender 5.2 Sky Texture SINGLE_SCATTERING parameter semantics.
// Physical density/optical coefficients follow the Earth atmosphere in
// Bruneton, Precomputed Atmospheric Scattering (2017), https://ebruneton.github.io/precomputed_atmospheric_scattering/.
// No Blender implementation code is used. Y is the scene's vertical axis.
struct Inputs {
    float sun_elevation = 0.261799;
    float sun_rotation = 0;
    [InputRange(0.0, 100000.0, true, true)] float altitude = 100;                 // metres, Blender 5.2
    [InputRange(0.0, 1000.0, true, true)] float air_density = 1;
    [InputRange(0.0, 1000.0, true, true)] float aerosol_density = 1;
    [InputRange(0.0, 1000.0, true, true)] float ozone_density = 1;
};
static const float SKY_PI = 3.141592653589793;
static const float SKY_GROUND = 6360000;  // Earth atmosphere reference radius, m
static const float SKY_TOP = 6460000;     // 100 km atmosphere
static const float3 SKY_RAYLEIGH = float3(5.802e-6,13.558e-6,33.100e-6);
static const float3 SKY_OZONE = float3(0.650e-6,1.881e-6,0.085e-6);
static const float SKY_MIE_SCATTER = 3.996e-6;
static const float SKY_MIE_EXTINCT = 4.440e-6;
static const float SKY_MIE_G = 0.8;
// Solar irradiance sampled at RGB wavelengths, integrated over visible bands.
static const float3 SKY_SOLAR = float3(147.4,185.04,191.198);
static const int SKY_VIEW_STEPS = 32;
static const int SKY_SUN_STEPS = 12;

float sky_exit(float3 o,float3 d,float radius) {
    float b=dot(o,d);
    float h=length(o)-radius;
    float delta=b*b-h*(length(o)+radius);
    if(delta<0) return -1;
    return -b+sqrt(delta);
}
bool sky_ground_hit(float3 o,float3 d) {
    float b=dot(o,d);
    float h=length(o)-SKY_GROUND;
    return b<0 && b*b-h*(length(o)+SKY_GROUND)>0;
}
float3 sky_density(float3 p,Inputs i) {
    float h=max(length(p)-SKY_GROUND,0);
    return float3(exp(-h/8000)*i.air_density,
        exp(-h/1200)*i.aerosol_density,
        max(1-abs(h-25000)/15000,0)*i.ozone_density);
}
float3 sky_extinction(float3 rho) {
    return SKY_RAYLEIGH*rho.x+SKY_MIE_EXTINCT*rho.y+SKY_OZONE*rho.z;
}
float3 sky_sun_transmission(float3 p,float3 sun,Inputs i) {
    if(sky_ground_hit(p,sun)) return float3(0);
    float distance=max(sky_exit(p,sun,SKY_TOP),0);
    float3 optical_depth=0;
    [loop] for(int k=0;k<SKY_SUN_STEPS;k++) {
        float a=float(k)/SKY_SUN_STEPS;
        float b=float(k+1)/SKY_SUN_STEPS;
        float t=distance*(a*a+b*b)*0.5;
        optical_depth+=sky_extinction(sky_density(p+sun*t,i))*distance*(b*b-a*a);
    }
    return exp(-optical_depth);
}
float3 environment(EnvironmentContext ctx, Inputs i) { float3 direction=ctx.direction;
    direction=normalize(direction);
    float3 o=float3(0,SKY_GROUND+max(i.altitude,1),0);
    // Blender 5.2.1 single scattering extends horizon radiance into the lower
    // hemisphere by max(1 - 2.5*sin(depression), 0)^3 (black at asin(0.4)).
    // Evaluate our own horizon transport; no ground or multiple scattering.
    float horizon_fade=pow(max(1+2.5*min(direction.y,0),0),3);

    if(horizon_fade==0) return float3(0);
    if(direction.y<0) direction=normalize(float3(direction.x,0,direction.z));
    // Blender +Y at rotation zero becomes -Z in the scene's Y-up frame.
    float3 sun=float3(cos(i.sun_elevation)*sin(i.sun_rotation),sin(i.sun_elevation),
        -cos(i.sun_elevation)*cos(i.sun_rotation));
    float distance=max(sky_exit(o,direction,SKY_TOP),0);
    float mu=dot(direction,sun);
    float rayleigh_phase=3*(1+mu*mu)/(16*SKY_PI);
    float g=SKY_MIE_G;
    float mie_phase=3*(1-g*g)*(1+mu*mu)/
        (8*SKY_PI*(2+g*g)*pow(max(1+g*g-2*g*mu,1e-6),1.5));
    float3 radiance=0;
    float3 transmission=1;
    [loop] for(int k=0;k<SKY_VIEW_STEPS;k++) {
        float a=float(k)/SKY_VIEW_STEPS;
        float b=float(k+1)/SKY_VIEW_STEPS;
        float dt=distance*(b*b-a*a);
        float3 p=o+direction*(distance*(a*a+b*b)*0.5);
        float3 rho=sky_density(p,i);
        float3 extinction=sky_extinction(rho);
        float3 t=exp(-extinction*dt);
        float3 integral=(1-t)/max(extinction,float3(1e-12));
        float3 scatter=SKY_RAYLEIGH*rho.x*rayleigh_phase+SKY_MIE_SCATTER*rho.y*mie_phase;
        radiance+=transmission*scatter*sky_sun_transmission(p,sun,i)*integral;
        transmission*=t;
    }
    radiance*=SKY_SOLAR*horizon_fade;
    radiance=max(radiance,float3(0));
    return radiance;
}
