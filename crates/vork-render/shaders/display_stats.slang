// Exposure report over the GPU colour output (display_stats.slangh).
import vork_constants;

[[vk::binding(0, 0)]] Texture2D<float4> display : register(t0);
[[vk::binding(1, 0)]] RWStructuredBuffer<uint> counters : register(u0);
#include "display_stats.slangh"

[shader("compute")]
[numthreads(REG_COMPUTE_TILE, REG_COMPUTE_TILE, 1)]
void reduce(uint3 id : SV_DispatchThreadID, uint lane : SV_GroupIndex)
{
    begin(lane);
    uint width = 0, height = 0;
    display.GetDimensions(width, height);
    if (all(id.xy < uint2(width, height))) count(display.Load(int3(id.xy, 0)));
    end(lane);
}
