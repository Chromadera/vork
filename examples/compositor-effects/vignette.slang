// Ellipse Mask + Gaussian Blur + multiply Mix. No geometric image remapping.
import vork;
public struct Inputs {
    float2 position = float2(0.5);
    [InputRange(0.0, 3.402823466e+38, true, true)] float2 size = float2(1.2,0.);
    float rotation = 0.;
    [InputRange(-1.0, 3.402823466e+38, true, true)] float2 blur_size = float2(-1);
    [InputRange(0.0, 1.0, true, true)] float factor = 0.4;
};
static const int VIGNETTE_BLUR_TAPS = 9;
public float ellipse_mask(float2 uv,float2 image_size,float2 position=float2(0.5),float2 size=float2(0.2,0.1),float rotation=0.) {
    float2 p=float2(uv.x,1.-uv.y)-position;
    p.y*=image_size.y/image_size.x;
    float cs=cos(rotation),sn=sin(rotation);
    // Rotate into the ellipse frame (inverse of the authored rotation).
    p=float2(cs*p.x+sn*p.y,-sn*p.x+cs*p.y);
    return dot(p/max(size*0.5,0.000001),p/max(size*0.5,0.000001))<=1. ? 1. : 0.;
}
public float vignette(float2 uv,float2 image_size,Inputs inputs) {
    // Zero height follows the frame aspect; -1 blur selects a quarter-frame radius.
    if(inputs.size.y==0.) inputs.size.y=inputs.size.x*image_size.y/image_size.x;
    inputs.blur_size=select(inputs.blur_size<0.,image_size*0.25,inputs.blur_size);
    if(all(inputs.blur_size<=0.)) return ellipse_mask(uv,image_size,inputs.position,inputs.size,inputs.rotation);
    float sum=0.,weight=0.;
    for(int y=0;y<VIGNETTE_BLUR_TAPS;y++) for(int x=0;x<VIGNETTE_BLUR_TAPS;x++) {
        float2 q=(float2(x,y)-4.)/4.;
        float w=exp(-4.5*dot(q,q));
        sum+=w*ellipse_mask(uv+q*inputs.blur_size/image_size,image_size,inputs.position,inputs.size,inputs.rotation);
        weight+=w;
    }
    return sum/weight;
}
public float4 composite(CompositeContext ctx, Inputs inputs) { float2 uv=ctx.uv; float2 image_size=ctx.size; float time=ctx.time;
    float4 c=compositor_sample(uv);
    return float4(c.rgb*lerp(1.,vignette(uv,image_size,inputs),saturate(inputs.factor)),c.a);
}
