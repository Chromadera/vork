import vork;
struct Inputs { Image beauty; Image halo; Image plate; float2 artifact_uv; float reveal=1.; };
float4 composite(CompositeContext ctx,Inputs i) {
    let plate=compositor_sample(i.plate,ctx.uv).rgb;
    let beauty=compositor_sample(i.beauty,ctx.uv).rgb;
    let halo=compositor_sample(i.halo,ctx.uv).rgb;
    let final=beauty+halo*.065;
    float amount=1.;
    if(i.reveal>0.) {
        float radius=max(0.,ctx.time-.8)*.85;
        float distance=length((ctx.uv-i.artifact_uv)*float2(ctx.frame_size.x/ctx.frame_size.y,1.));
        amount=ctx.time<=.8?0.:1.-smoothstep(radius-.003,radius+.003,distance);
    }
    return float4(lerp(plate,final,amount),1.);
}
