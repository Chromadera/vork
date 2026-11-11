// Diagonal paint stripes in world space over an optional albedo image;
// bare metal where the paint is missing.
import vork;

struct Inputs {
    float3 paint = float3(0.85, 0.32, 0.08);
    float3 metal = float3(0.9, 0.9, 0.92);
    float frequency = 6.0;
    float coverage = 0.6;
    float speed = 0.0;  // stripe scroll, cycles per second
    Image albedo;
};

Closure surface(ShadingContext ctx, Inputs inputs) {
    Closure c = {};
    float s = frac((ctx.POSITION.x + ctx.POSITION.y + ctx.POSITION.z) * inputs.frequency * 0.5 + ctx.time * inputs.speed);
    bool painted = s < inputs.coverage;
    float3 albedo = sample(inputs.albedo, ctx.TEXCOORD_0, ctx).rgb;
    c.base_color = painted ? inputs.paint * albedo : inputs.metal;
    c.base_metalness = painted ? 0.0 : 1.0;
    c.specular_roughness = painted ? 0.55 : 0.18;
    return c;
}
