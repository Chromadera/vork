import vork;
struct Inputs {
    Image plate; Image catcher; Image cg; Image hand;
};
void sort_pair(inout float3 a,inout float3 b) { let low=min(a,b);b=max(a,b);a=low; }
float3 clean_ratio(CompositeContext ctx,Image image) {
    let pixel=1./ctx.frame_size;
    float3 a=compositor_sample(image,ctx.uv).rgb;
    float3 b=compositor_sample(image,ctx.uv+float2(pixel.x,0.)).rgb;
    float3 c=compositor_sample(image,ctx.uv-float2(pixel.x,0.)).rgb;
    float3 d=compositor_sample(image,ctx.uv+float2(0.,pixel.y)).rgb;
    float3 e=compositor_sample(image,ctx.uv-float2(0.,pixel.y)).rgb;
    sort_pair(a,b);sort_pair(d,e);sort_pair(a,c);sort_pair(b,c);sort_pair(a,d);
    sort_pair(c,d);sort_pair(b,e);sort_pair(b,c);sort_pair(d,e);
    return clamp(c,float3(.08),float3(32.));
}
float4 composite(CompositeContext ctx,Inputs i) {
    float4 cg=compositor_sample(i.cg,ctx.uv);
    let plate=compositor_sample(i.plate,ctx.uv).rgb;
    let catcher=clean_ratio(ctx,i.catcher);

    // Single-view relief cannot recover the thin wrist/desk gap reliably.
    // Keep the photographed hand and its boundary; only relight receivers
    // outside the hand and its softly expanded lower boundary. Occlusion still uses geometry.
    float hand=0.; float weights=0.;
    for(int y=0;y<=4;y++) for(int x=-3;x<=3;x++) {
        let w=exp(-float(x*x+y*y)/4.);
        hand+=compositor_sample(i.hand,ctx.uv+float2(x,-y)*12./ctx.frame_size).r*w;
        weights+=w;
    }
    let protection=smoothstep(.005,.08,hand/weights);
    let lit_plate=plate*lerp(catcher,float3(1.),protection);
    float3 result=cg.rgb+lit_plate*(1.-cg.a);
    return float4(max(result,float3(0.)),1.);
}
