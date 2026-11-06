public struct CompositeContext {
    public float2 uv;
    public float2 size;
    public float time;
    public float2 frame_size;
    public float2 output_size;
    public float4x4 view;
    public float4x4 projection;
    public float aperture_radius;
    public float focus_distance;
    public bool projective;
    public uint aperture_blades;
    public float aperture_rotation;
    public float aperture_ratio;
    public float2 clip;
    public uint frame;
    public uint samples;
    public float2 shutter;
};
float4 composite_context_word(uint index) {
    __target_switch { case wgsl: __intrinsic_asm "vork_compositor_context($0)"; }
}
public CompositeContext vork_composite_context(float2 uv, float2 size, float time) {
    CompositeContext ctx = {};
    ctx.uv=uv; ctx.size=size; ctx.time=time;
    ctx.view=transpose(float4x4(composite_context_word(0),composite_context_word(1),composite_context_word(2),composite_context_word(3)));
    ctx.projection=transpose(float4x4(composite_context_word(4),composite_context_word(5),composite_context_word(6),composite_context_word(7)));
    float4 optics=composite_context_word(8);
    float4 sensor=composite_context_word(9);
    float4 aperture=composite_context_word(10);
    float4 clip=composite_context_word(11);
    float4 sampling=composite_context_word(12);
    float4 shutter=composite_context_word(13);
    float4 extent=composite_context_word(15);
    ctx.frame_size=extent.xy;ctx.output_size=extent.zw;
    ctx.aperture_radius=optics.x;ctx.focus_distance=optics.z;ctx.projective=optics.w!=0.;
    ctx.aperture_blades=uint(aperture.x);ctx.aperture_rotation=aperture.y;ctx.aperture_ratio=aperture.z;
    ctx.clip=clip.xy;ctx.frame=uint(sampling.x);ctx.samples=uint(sampling.y);
    ctx.shutter=shutter.xy;
    return ctx;
}
