// The comparison reveal is also a vork compositor program. No video-editor wipe.
import vork;
struct Inputs { Image source; Image plate; int mode=0; };
float4 composite(CompositeContext ctx,Inputs i) {
    let beauty=compositor_sample(i.source,ctx.uv).rgb;
    if(i.mode==0)return float4(beauty,1.);
    let plate=compositor_sample(i.plate,ctx.uv).rgb;
    let edge=saturate((ctx.time-.5)/1.5)*1.02-.01;
    float3 rgb=ctx.uv.x<edge?beauty:plate;
    if(edge>0. && edge<1.) {
        let d=abs(ctx.uv.x-edge)*ctx.frame_size.x;
        rgb+=float3(.12,.72,1.)*(exp(-d*d/1.2)*1.4+exp(-d/5.)*.12);
    }
    return float4(rgb,1.);
}
