import vork;
struct Inputs { Image person; };
Closure surface(ShadingContext ctx, Inputs i) {
    Closure c = {};
    c.base_color = float3(.55, .42, .33);
    c.specular_weight = 0.;
    c.geometry_opacity = sample(i.person, ctx.TEXCOORD_0, ctx).r;
    return c;
}
