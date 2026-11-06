import vork;
struct Inputs { Image beauty; Image plate; };
float4 composite(CompositeContext ctx,Inputs i) {
    // Broad, low-energy lens scatter. This node runs at quarter resolution;
    // sample a fixed angular neighborhood from the full-resolution beauty.
    float3 result=0.; float weight=0.;
    for(int y=-4;y<=4;y++) for(int x=-4;x<=4;x++) {
        let w=exp(-float(x*x+y*y)/8.);
        let uv=ctx.uv+float2(x,y)*.004;
        let c=compositor_sample(i.beauty,uv).rgb-compositor_sample(i.plate,uv).rgb;
        result+=max(c-float3(.8),float3(0.))*w;weight+=w;
    }
    return float4(result/weight,1.);
}
