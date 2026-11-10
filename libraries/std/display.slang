import vork;
// Blender 5.2.1 display gamma and triangular output dither.
public float3 finish_display(float3 color, DisplayContext ctx, float gamma, float dither_intensity) {
    if (gamma != 1.) color = pow(max(color, float3(0.)), float3(1. / gamma));
    uint seed = ctx.pixel.x + ctx.pixel.y * 0x9e3779b9u;
    seed = (seed ^ (seed >> 16u)) * 0x45d9f3bu;
    seed = (seed ^ (seed >> 16u)) * 0x45d9f3bu;
    seed ^= seed >> 16u;
    float u = float(seed) * 2.3283064365386963e-10 * 2. - 1.;
    return color + sign(u) * (1. - sqrt(1. - abs(u))) * 0.0033 * (255. / (exp2(float(ctx.bit_depth)) - 1.)) * dither_intensity;
}
