import vork;
import helpers;

struct Inputs {
    float3 tint = float3(0.5, 0.2, 0.1);
    float roughness = 0.4;
    float scale = 3.0;
    bool glossy = true;
    int bands = 2;
    Image albedo;
};

Closure surface(ShadingContext ctx, Inputs inputs) {
    Closure c = {};
    float s = stripe(ctx.TEXCOORD_0.x * inputs.scale * inputs.bands);
    c.base_color = inputs.tint * s * sample(inputs.albedo, ctx.TEXCOORD_0, ctx).rgb;
    if (inputs.glossy)
        c.specular_roughness = inputs.roughness;
    return c;
}
