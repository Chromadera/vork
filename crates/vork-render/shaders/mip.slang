// Material mip generation: one full-screen triangle samples the previous level.
[[vk::binding(0, 0)]] Texture2D source : register(t0);
[[vk::binding(1, 0)]] SamplerState smp : register(s0);

struct V
{
    float4 p : SV_Position;
    float2 uv : TEXCOORD0;
};

[shader("vertex")]
// Vulkan's VertexIndex as is: SV_VertexID subtracts BaseVertex and needs DrawParameters.
#ifdef VORK_METAL
V vs(uint i: SV_VertexID)
#else
V vs(uint i: SV_VulkanVertexID)
#endif
{
    float2 p = float2(float((i << 1u) & 2u), float(i & 2u));
    V v={};
    v.p = float4(p * float2(2., -2.) + float2(-1., 1.), 0., 1.);
    v.uv = p;
    return v;
}

[shader("fragment")]
float4 fs(V v) : SV_Target
{
    uint width,height;source.GetDimensions(width,height);
    let size=uint2(width,height);
    // Exact halves (and collapsed axes) are one hardware bilinear average.
    if ((width==1u || width%2u==0u) && (height==1u || height%2u==0u)) {
        return source.SampleLevel(smp,v.uv,0.);
    }
    // Odd reductions span more than two source texels. Integrate their actual
    // area, preserving scene-linear premultiplied colour and alpha together.
    let target=max(size/2u,uint2(1u));
    let scale=float2(size)/float2(target);
    let lo=floor(v.p.xy)*scale;let hi=min(lo+scale,float2(size));
    let start=uint2(floor(lo));let end=min(uint2(ceil(hi)),size);
    var sum=float4(0.);
    for(var y=start.y;y<end.y;y++) {
        for(var x=start.x;x<end.x;x++) {
            let overlap=min(hi,float2(x+1u,y+1u))-max(lo,float2(x,y));
            sum+=source.Load(int3(x,y,0))*overlap.x*overlap.y;
        }
    }
    return sum/((hi.x-lo.x)*(hi.y-lo.y));
}
