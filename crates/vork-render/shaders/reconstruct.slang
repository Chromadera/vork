// Stateless, scene-linear edge-aware wavelet reconstruction. No history,
// motion reprojection, host image transfer or external runtime.
import vork_constants;
struct Params { uint width; uint height; uint samples; uint step; };
[[vk::binding(0,0)]] ConstantBuffer<Params> params : register(b0);
[[vk::binding(1,0)]] RWStructuredBuffer<float4> accumulation : register(u1);
[[vk::binding(2,0)]] StructuredBuffer<float4> guides : register(t2);
[[vk::binding(3,0)]] StructuredBuffer<float4> source : register(t3);
[[vk::binding(4,0)]] RWStructuredBuffer<float4> destination : register(u4);
[[vk::binding(5,0)]] RWStructuredBuffer<float4> filtered_guides : register(u5);
uint index(int2 p) { p=clamp(p,int2(0),int2(params.width,params.height)-1); return uint(p.y)*params.width+uint(p.x); }
float3 radiance(uint i) { let p=accumulation[i]; return p.a>0. ? p.rgb/p.a : float3(0.); }
float3 guide(uint i,uint plane) { let a=guides[2*i].a; return a>0. ? guides[2*i+plane].rgb/a : float3(0.); }
[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void prepare(uint3 id:SV_DispatchThreadID) {
    if(id.x>=params.width || id.y>=params.height) return;
    let i=index(int2(id.xy)); var mean=float3(0.);var square=float3(0.);
    for(int y=-1;y<=1;y++) for(int x=-1;x<=1;x++) {
        let j=index(int2(id.xy)+int2(x,y));let c=radiance(j);mean+=c;square+=c*c;
    }
    mean/=9.;
    let variance=max(square/9.-mean*mean,float3(0.));
    destination[i]=float4(radiance(i),dot(variance,float3(1./3.)));
    // Do not preblur material edges. A noisy guide is safer than invented
    // agreement across two receivers or a texture boundary.
    filtered_guides[2*i]=float4(guide(i,0),0.);
    filtered_guides[2*i+1]=float4(guide(i,1),guides[2*i+1].w);
}
[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void filter(uint3 id:SV_DispatchThreadID) {
    if(id.x>=params.width || id.y>=params.height) return;
    let i=index(int2(id.xy));let center=source[i];let a=filtered_guides[2*i].rgb;let n=filtered_guides[2*i+1].rgb;
    let identity=filtered_guides[2*i+1].w;
    // Decoupled camera coverage can extend radiance beyond the representative
    // control sample's surface. Those un-guided silhouette pixels are signal,
    // not background noise: coarse taps would spread them into a wavelet grid.
    // Identity zero denotes no surface; negative identities protect transport
    // and mixed coverage. Reuse that word rather than load raw guide coverage.
    if(identity<=0.) {destination[i]=center;return;}
    let coverage=accumulation[i].a/float(params.samples);
    if(coverage<=0.) {destination[i]=center;return;}
    var sum=center.rgb;var variance=center.a;
    for(int y=-1;y<=1;y++) for(int x=-1;x<=1;x++) {
        let position=int2(id.xy)+int2(x,y)*int(params.step);
        // Clamped border taps are not reciprocal. Missing neighbours exchange
        // no energy, leaving their weight at the receiver itself.
        if(any(position<int2(0)) || any(position>=int2(params.width,params.height))) {continue;}
        let j=index(position);let tap=source[j];
        if(filtered_guides[2*j+1].w!=identity) {continue;}
        let da=filtered_guides[2*j].rgb-a;let dn=filtered_guides[2*j+1].rgb-n;
        if(dot(da,da)>RECONSTRUCT_ALBEDO_EDGE_SQUARED) {continue;}
        let dc=length(tap.rgb-center.rgb)/sqrt(3.);
        let tap_coverage=accumulation[j].a/float(params.samples);
        if(tap_coverage<=0.) {continue;}
        let alpha=tap_coverage-coverage;
        let spatial=float((x==0?2:1)*(y==0?2:1));
        let sigma=RECONSTRUCT_COLOR_SIGMA*sqrt(max(center.a+tap.a,RECONSTRUCT_VARIANCE_FLOOR));
        let weight=spatial*exp(-dot(da,da)*RECONSTRUCT_ALBEDO_SCALE-dot(dn,dn)*RECONSTRUCT_NORMAL_SCALE-abs(dc)/sigma-abs(alpha)*RECONSTRUCT_ALPHA_SCALE);
        // Off-centre spatial weights sum to 12; the centre exchanges zero.
        // Every pair uses the same conductance
        // and exchanges opposite premultiplied RGB flux; do not normalize each
        // receiver by a different surviving weight sum. min coverage keeps the
        // update convex even at fractional camera coverage.
        let exchange=weight*(min(coverage,tap_coverage)/coverage)/12.;
        sum+=(tap.rgb-center.rgb)*exchange;variance+=(tap.a-center.a)*exchange;
    }
    // Keep the local noise scale through coarse levels. Shrinking it after
    // every pass mistakes residual low-frequency noise for a lighting edge
    // and freezes moving blotches on otherwise smooth walls.
    destination[i]=float4(sum,variance);
}
[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void finish(uint3 id:SV_DispatchThreadID) {
    if(id.x>=params.width || id.y>=params.height) return;
    let i=index(int2(id.xy));let original=accumulation[i]/float(params.samples);
    // Preserve alpha and exact empty-background pixels. Update the accumulated
    // source as well so compositor inputs and float outputs see the same image.
    let p=float4(original.a>0. ? source[i].rgb*original.a : original.rgb,original.a);
    destination[i]=p;accumulation[i]=p*float(params.samples);
}
