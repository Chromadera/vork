// Physical point coefficients; stage 1 path transport.
import vork;
struct Inputs {
    float3 absorption = float3(0.0);
    float3 scattering = float3(0.5);
    float3 emission = float3(0.0);
    float anisotropy = 0.0;
};
MediumCoefficients medium(MediumContext ctx, Inputs inputs) {
    MediumCoefficients m;
    m.absorption = inputs.absorption;
    m.scattering = inputs.scattering;
    m.emission = inputs.emission;
    m.anisotropy = inputs.anisotropy;
    return m;
}
