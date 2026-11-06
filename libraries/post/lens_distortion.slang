// Lens Distortion's dispersion only. The green channel remains at the camera UV.
import vork;
public struct Inputs { [InputRange(0.0, 1.0, true, true)] float dispersion = 0.; };
public float4 composite(CompositeContext ctx, Inputs inputs) { float2 uv=ctx.uv; float2 size=ctx.size; float time=ctx.time;
    float4 center=compositor_sample(uv);
    if(inputs.dispersion<=0.) return center;
    float2 p=(uv-.5)*2.;
    float radius2=dot(p,p);
    float amount=clamp(inputs.dispersion,0.,1.);
    // Differential radial lens magnification; no common geometric distortion.
    float red_scale=1./(1.+sqrt(max(1.-amount*radius2,0.)));
    float blue_scale=1./(1.+sqrt(1.+amount*radius2));
    float4 red=compositor_sample(.5+p*red_scale);
    float4 blue=compositor_sample(.5+p*blue_scale);
    float r=red.a>0. ? red.r/red.a : 0.;
    float b=blue.a>0. ? blue.b/blue.a : 0.;
    return float4(r*center.a,center.g,b*center.a,center.a);
}
