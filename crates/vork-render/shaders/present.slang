// Display-encoded colour over black; the attachment must not encode sRGB again.
[[vk::binding(0, 0)]] Texture2D source : register(t0);
[[vk::binding(1, 0)]] SamplerState smp : register(s0);
struct V { float4 p : SV_Position; float2 uv : TEXCOORD0; };
[shader("fragment")]
float4 fs(V v) : SV_Target {
    return float4(source.SampleLevel(smp, v.uv, 0.).rgb, 1.);
}
