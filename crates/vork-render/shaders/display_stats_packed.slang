// Exposure report over the packed RGBA8 colour readback plane (display_stats.slangh):
// `size.x` words from the start of `packed`, reduced by rows of `size.y` workgroups
// (dispatch dimensions are limited).
import vork_constants;

[[vk::binding(0, 0)]] StructuredBuffer<uint> packed : register(t0);
[[vk::binding(1, 0)]] RWStructuredBuffer<uint> counters : register(u1);
struct Size { uint4 value; };
[[vk::binding(2, 0)]] ConstantBuffer<Size> size : register(b2);
#include "display_stats.slangh"

[shader("compute")]
[numthreads(64, 1, 1)]
void reduce(uint3 group : SV_GroupID, uint lane : SV_GroupIndex)
{
    begin(lane);
    uint i = (group.y * size.value.y + group.x) * 64u + lane;
    if (i < size.value.x) {
        uint word = packed[i];
        count(float4(word & 0xffu, (word >> 8u) & 0xffu, (word >> 16u) & 0xffu, word >> 24u) / 255.);
    }
    end(lane);
}
