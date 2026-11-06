import vork;
struct Inputs { Image source; Image plate; Image person; float2 direction=float2(1.,0.); };
float4 composite(CompositeContext ctx,Inputs i) {
    float3 sum=0.;float weight=0.;
    let center=compositor_sample(i.plate,ctx.uv).rgb;
    let mask=compositor_sample(i.person,ctx.uv).r;
    for(int k=-6;k<=6;k++) {
        let uv=ctx.uv+i.direction*float(k)/ctx.size;
        let guide=compositor_sample(i.plate,uv).rgb;
        let matte=compositor_sample(i.person,uv).r;
        let w=exp(-float(k*k)/12.-dot(guide-center,guide-center)*14.-abs(matte-mask)*8.);
        sum+=clamp(compositor_sample(i.source,uv).rgb,float3(.08),float3(32.))*w;
        weight+=w;
    }
    return float4(sum/max(weight,.00001),1.);
}
