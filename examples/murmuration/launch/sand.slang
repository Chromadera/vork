import vork;
struct Inputs {};
Closure surface(ShadingContext c, Inputs i) {
 float3 p=c.POSITION;
 float r=length(float2((p.x-13.)/6.,(p.z+2.)/29.));
 float wet=smoothstep(.57,.97,r);
 float ripple=.025*sin(p.x*42.+sin(p.z*3.1)*2.)+.016*sin(p.z*77.+p.x*21.);
 Closure m={};m.base_color=lerp(float3(.29,.22,.13),float3(.065,.055,.038),wet)*(1.+ripple);
 m.specular_roughness=lerp(.9,.38,wet);return m;
}
