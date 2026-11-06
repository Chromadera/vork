// Walnut garden table with a worn, wettable finish, in world metres.
import vork;
static const float METRES_PER_UNIT=1./6.;
static const float DOMAIN=3.;
import std.procedurals;
import std.grid;
struct Inputs { FloatGrid wetness; };
Closure surface(ShadingContext c,Inputs i) {
    Closure m={};float3 p=c.POSITION;
    float grain=noise(p*float3(.35,3.,20.),9.,3.);
    float longgrain=noise(p*float3(.15,1.,8.),4.,3.);
    float board=floor((p.z+10.)/.32);
    float tint=.85+.15*sin(board*7.13);
    float seam=smoothstep(.008,.014,min(frac((p.z+10.)/.32),1.-frac((p.z+10.)/.32))*.32);
    m.base_color=lerp(float3(.045,.016,.006),float3(.24,.11,.035),.6*grain+.4*longgrain)*tint;
    m.base_color*=lerp(.25,1.,seam);
    float wet=smoothstep(.00003,.0003,METRES_PER_UNIT*sample_grid(i.wetness,float3(p.x/(DOMAIN*METRES_PER_UNIT),0.,p.z/(DOMAIN*METRES_PER_UNIT))));
    m.base_color*=lerp(1.,.55,wet);
    m.specular_roughness=lerp(.3+.17*grain,.08,wet);
    m.coat_weight=wet;m.coat_ior=1.333;m.coat_roughness=.025;
    return m;
}
