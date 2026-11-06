import vork;
import std.display;
public struct Inputs {
    float exposure = 0.;
    [InputRange(0., 3.402823466e38, false, true)] float gamma = 1.;
    [InputRange(0., 3.402823466e38, true, true)] float dither_intensity = 1.;
};
public float3 display(DisplayContext ctx, Inputs inputs) {
    float3 c = ctx.color * exp2(inputs.exposure);
    float3 mapped = select(c > float3(.0031308), 1.055 * pow(max(c, float3(0.)), float3(1. / 2.4)) - .055, 12.92 * c);
    return finish_display(mapped, ctx, inputs.gamma, inputs.dither_intensity);
}
