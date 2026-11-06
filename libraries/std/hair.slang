// Blender 5.2.1 Principled Hair, Chiang sockets.
// Defaults audited from Blender 5.2.1 9e2066aef7ef; the Inputs initializers below own the socket contract.
import vork;
import std.hair_bsdf;
enum HairParametrization { Color, Melanin, Absorption };
struct Inputs {
    HairParametrization parametrization = HairParametrization::Color;
    float3 color = float3(0.017513,0.005763,0.002059);
    float melanin = 0.8;
    float melanin_redness = 1.0;
    float3 tint = float3(1.0);
    float3 absorption_coefficient = float3(0.245531,0.52,1.365);
    float roughness = 0.3;
    float radial_roughness = 0.3;
    float coat = 0.0;
    float ior = 1.55;
    float offset = 0.034906585;
    float random_roughness = 0.0;
    float random_color = 0.0;
    float random = 0.0;
};
Hair chiang(ShadingContext ctx, Inputs i) {
    if (i.parametrization==HairParametrization::Melanin)
        return principled_hair_melanin(i.melanin,i.melanin_redness,i.tint,
            i.roughness,i.radial_roughness,i.coat,i.ior,i.offset,
            i.random_color,i.random_roughness,i.random);
    let factor=1.+(2.*clamp(i.random,0.,1.)-1.)*clamp(i.random_roughness,0.,1.);
    if (i.parametrization==HairParametrization::Absorption)
        return principled_hair_absorption(i.absorption_coefficient,
            clamp(i.roughness*factor,0.,1.),clamp(i.radial_roughness*factor,0.,1.),
            i.coat,i.ior,i.offset);
    return principled_hair(i.color,clamp(i.roughness*factor,0.,1.),
                          clamp(i.radial_roughness*factor,0.,1.),i.coat,i.ior,i.offset);
}

Hair surface(ShadingContext ctx, Inputs i) {
    var h=chiang(ctx,i);
    // Retain Color for custom diffuse consumers; physical scattering uses absorption.
    h.color=max(i.color,float3(0.));
    h.scattering = true;
    return h;
}
