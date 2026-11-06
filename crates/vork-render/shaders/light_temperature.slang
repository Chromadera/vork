// Light temperature: std.color blackbody at every kelvin of Blender's Light.temperature
// range, read back once per install (src/light_temperature.rs). The table is padded to
// whole workgroups, so every invocation has an entry (Metal has no GetDimensions here).
import std.color;

[[vk::binding(0, 0)]] RWStructuredBuffer<float4> table : register(u0);

[shader("compute")]
[numthreads(64, 1, 1)]
void tabulate(uint3 id : SV_DispatchThreadID)
{
    table[id.x] = float4(blackbody(800. + float(id.x)), 0.);
}
