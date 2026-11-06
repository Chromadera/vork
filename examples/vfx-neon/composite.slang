// Example-owned compositor: ordered named images, multi-resolution glow and wipe.
import vork;
struct Inputs {
    int mode = 0;
    Image source;
    Image plate;
    Image catcher;
    Image integrated;
    Image glow;
    Image wide;
    float2 direction = float2(1.,0.);
    float wipe = 1.;
};
uint hash(uint x) {x^=x>>16;x*=0x7feb352du;x^=x>>15;x*=0x846ca68bu;return x^(x>>16);}
float noise(uint x) {return float(hash(x)&0xffffffu)/16777216.-.5;}
void order(inout float3 a,inout float3 b) {let lo=min(a,b);b=max(a,b);a=lo;}
float3 ratio(Inputs i,float2 uv,float2 pixel) {
    float3 a=compositor_sample(i.catcher,uv).rgb;
    float3 b=compositor_sample(i.catcher,uv+float2(pixel.x,0)).rgb;
    float3 c=compositor_sample(i.catcher,uv-float2(pixel.x,0)).rgb;
    float3 d=compositor_sample(i.catcher,uv+float2(0,pixel.y)).rgb;
    float3 e=compositor_sample(i.catcher,uv-float2(0,pixel.y)).rgb;
    order(a,b);order(d,e);order(a,c);order(b,c);order(a,d);
    order(c,d);order(b,e);order(b,c);order(d,e);return c;
}
float4 composite(CompositeContext ctx,Inputs i) {
    let uv=ctx.uv;
    if(i.mode==0) return compositor_sample(uv);
    if(i.mode==5) return float4(ratio(i,uv,1./ctx.frame_size),1.);
    if(i.mode==1) {
        let cg=compositor_sample(i.source,uv);
        let plate=compositor_sample(i.plate,uv).rgb;
        let raw_ratio=compositor_sample(i.catcher,uv).rgb;
        let catcher_ratio=1.+(raw_ratio-1.)*.12;
        // Apply the highlight shoulder only to the luminous object.
        let peak=max(cg.r,max(cg.g,cg.b))/max(cg.a,.00001);
        let neon=cg.rgb/(1.+peak*.55);
        // Solid tubes and supports cover the plate with premultiplied alpha.
        // Artistic dark-ground light floor, not recovered additive HDR radiance.
        let floor_boost=.015*smoothstep(.60,.65,uv.y);
        let relit=plate*min(catcher_ratio,float3(1.))+max(plate,float3(floor_boost))*max(catcher_ratio-1.,float3(0.));
        let rgb=neon+relit*(1.-cg.a);
        return float4(max(rgb,float3(0.)),1.);
    }
    if(i.mode==2) {
        let cg=compositor_sample(i.source,uv);
        let added=max(compositor_sample(i.integrated,uv).rgb-compositor_sample(i.plate,uv).rgb,float3(0.));
        return float4(min(cg.rgb,float3(12.))+added*.5,1.);
    }
    if(i.mode==3) {
        float3 sum=float3(0.);float weights=0.;
        for(int k=-8;k<=8;k++) {
            float w=exp(-float(k*k)/20.);
            sum+=compositor_sample(uv+i.direction*float(k)/ctx.size).rgb*w;
            weights+=w;
        }
        return float4(sum/weights,1.);
    }
    let plate=compositor_sample(i.plate,uv).rgb;
    let base=compositor_sample(i.integrated,uv).rgb;
    let glow=compositor_sample(i.glow,uv).rgb*.065+compositor_sample(i.wide,uv).rgb*.020;
    let pixel=uint2(uv*ctx.frame_size);
    let grain=noise(pixel.x+pixel.y*uint(ctx.frame_size.x)+hash(ctx.frame))*min(length(glow),.25)*.012;
    float3 result=base+glow+grain;
    if(uv.y<127./1080. || uv.y>=952./1080.) result=float3(0.);
    if(i.wipe>.5) {
        // Untouched plate for the first half-second; reveal left to right.
        let edge=saturate((ctx.time-.5)/1.5)*1.02-.01;
        result=uv.x<edge?result:plate;
        if(edge>0. && edge<1.) {
            let distance=abs(uv.x-edge)*ctx.frame_size.x;
            result+=float3(.12,.72,1.)*(exp(-distance*distance/1.2)*1.4+exp(-distance/5.)*.12);
        }
    }
    return float4(max(result,float3(0.)),1.);
}
