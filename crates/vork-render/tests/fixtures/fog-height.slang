// World-height physical coefficients; stage 1 path transport.
// Coefficients at world height base_height; falloff is inverse world metres.
// Emission is independent of extinction; a divergent infinite source is refused.
import vork;
struct Inputs {
    float3 absorption = float3(0.0);
    float3 scattering = float3(0.5);
    float3 emission = float3(0.0);
    float anisotropy = 0.0;
    float base_height = 0.0;
    float falloff = 1.0;
};
// Preserve vacuum channels even where the exponential would overflow. The
// renderer still refuses nonfinite positive channels at an evaluated point.
float scaled_coefficient(float coefficient, float scale) {
    return coefficient == 0.0 ? 0.0 : coefficient * scale;
}
float3 scaled_coefficients(float3 coefficient, float scale) {
    return float3(scaled_coefficient(coefficient.x, scale),
        scaled_coefficient(coefficient.y, scale), scaled_coefficient(coefficient.z, scale));
}
MediumCoefficients medium(MediumContext ctx, Inputs inputs) {
    float scale = exp(-inputs.falloff * (ctx.world_position.y - inputs.base_height));
    MediumCoefficients m;
    m.absorption = scaled_coefficients(inputs.absorption, scale);
    m.scattering = scaled_coefficients(inputs.scattering, scale);
    m.emission = inputs.emission;
    m.anisotropy = inputs.anisotropy;
    return m;
}
