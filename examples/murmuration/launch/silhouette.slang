import vork;
struct Inputs {};
float3 displace(DisplacementContext c, Inputs i) {
 float3 p=c.POSITION;float wing=smoothstep(.006,.014,abs(p.x));
 float active=1.-smoothstep(.65,.77,frac(c.time/3.7))+smoothstep(.92,1.,frac(c.time/3.7));
 float a=(.12+.78*sin(c.time*6.2*6.2831853))*active;
 return float3(p.x*(cos(a)-1.),abs(p.x)*sin(a),0)*wing;
}
Closure surface(ShadingContext c, Inputs i) {
 Closure m={};m.base_color=float3(.018,.013,.022);m.specular_roughness=.5;return m;
}
