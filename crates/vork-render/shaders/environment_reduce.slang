// Area filter equirectangular radiance in solid angle. Partial source texels
// matter for non-power-of-two HDRs; latitude area is the cosine difference.
import vork_constants;

[[vk::binding(0, 0)]] Texture2DArray<uint4> source : register(t0);
[[vk::binding(1, 0)]]
[[vk::image_format("rgba32f")]] RWTexture2D<float4> output : register(u1);

[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void reduce_environment(uint3 id : SV_DispatchThreadID)
{
    uint width = 0, height = 0;
    output.GetDimensions(width, height);
    uint2 size = uint2(width, height);
    if (any(id.xy >= size)) return;
    uint source_width = 0, source_height = 0, source_layers=0;
    source.GetDimensions(source_width, source_height,source_layers);
    float2 source_size = float2(source_width, source_height);
    float2 low = float2(id.xy) / float2(size) * source_size;
    float2 high = float2(id.xy + 1u) / float2(size) * source_size;
    float3 mean = float3(0.);
    float total = 0.;
    for (uint y = uint(floor(low.y)); y < min(uint(ceil(high.y)), source_height); y++)
    {
        float a = max(float(y), low.y) / source_size.y;
        float b = min(float(y + 1u), high.y) / source_size.y;
        float latitude = 2. * sin(REG_PI * (a + b) * .5) * sin(REG_PI * (b - a) * .5);
        for (uint x = uint(floor(low.x)); x < min(uint(ceil(high.x)), source_width); x++)
        {
            float weight = latitude * (min(float(x + 1u), high.x) - max(float(x), low.x));
            // Rounded interval endpoints can include a zero-area texel.
            // Do not form 0/0 on the first contribution.
            if (weight <= 0.) continue;
            total += weight;
            let value=asfloat(source.Load(int4(int(x), int(y),1, 0)));
            mean += (value.rgb - mean) * (weight / total);
        }
    }
    output[id.xy] = float4(mean, 1.);
}
