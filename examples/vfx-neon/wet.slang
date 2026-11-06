// Example-owned uneven wet brick response. Coordinates are scene metres.
import vork;
struct Inputs {float3 tint=float3(.065,.055,.05);float wetness=1.;};
Closure surface(ShadingContext ctx,Inputs i) {
    Closure c={};
    float2 p=ctx.POSITION.xz;
    float patch=sin(p.x*1.7+sin(p.y*.7))*sin(p.y*2.1);
    c.base_color=i.tint;
    c.specular_roughness=lerp(.095,.035,smoothstep(-.2,.65,patch)*i.wetness);
    c.specular_weight=1.;
    // Sub-millimetre travelling water undulations, visible in new reflections.
    float a=p.x*77.+p.y*33.+ctx.time*2.3;
    float b=p.x*131.-p.y*91.-ctx.time*3.4;
    c.geometry_normal=normalize(float3(-.006*cos(a)-.003*cos(b),1.,-.003*cos(a)+.002*cos(b)));
    return c;
}
