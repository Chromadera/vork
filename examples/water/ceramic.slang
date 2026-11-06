import vork;
static const float METRES_PER_UNIT=1./6.;
struct Inputs {};
Closure surface(ShadingContext c,Inputs i) {
 Closure m={};float3 p=c.POSITION/METRES_PER_UNIT;
 float2 q=p.xz/.105;float2 f=frac(q);float edge=min(min(f.x,1.-f.x),min(f.y,1.-f.y));
 float grout=1.-smoothstep(.018,.035,edge);
 float checker=fmod(abs(floor(q.x)+floor(q.y)),2.);
 float3 tile=lerp(float3(.57,.65,.58),float3(.30,.43,.44),checker);
 float floorMask=(1.-smoothstep(-.86,-.82,p.y))*(1.-smoothstep(.81,.83,length(p.xz)));
 m.base_color=lerp(float3(.57,.65,.58),lerp(tile,float3(.19,.20,.18),grout),floorMask);
 m.specular_roughness=.23;m.coat_weight=.3;return m;
}