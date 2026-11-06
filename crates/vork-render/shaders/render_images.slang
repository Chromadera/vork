// Publish the scene-linear planar float color into a render image.
import vork_constants;

[[vk::binding(0, 0)]] StructuredBuffer<uint> pixels : register(t0);
[[vk::binding(1, 0)]]
[vk::image_format("rgba16f")]
RWTexture2D<float4> image_output : register(u0);

[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void publish(uint3 gid : SV_DispatchThreadID)
{
    uint width, height;
    image_output.GetDimensions(width, height);
    if (any(gid.xy >= uint2(width, height))) { return; }
    uint count = width * height;
    uint index = gid.y * width + gid.x;
    float4 rgba;
    for (uint channel = 0u; channel < 4u; channel++) {
        rgba[channel] = asfloat(pixels[(REG_FLOAT_COLOR_PLANE + channel) * count + index]);
    }
    image_output[gid.xy] = rgba;
}
