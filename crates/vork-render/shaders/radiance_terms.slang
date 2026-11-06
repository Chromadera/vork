// Scene-linear outgoing terms. Diffuse contains total received irradiance
// (local transport plus distant visibility), never glossy radiance.
public struct RadianceTerms {
    public float3 direct; public float3 emission; public float3 diffuse_indirect; public float3 glossy_indirect;
    public float3 total() {return direct+emission+diffuse_indirect+glossy_indirect;}
};
// Exact shaded terms owned by later provider substitution. They are weighted
// with the receiver's coverage and transmittance, just like colour.
public struct ReplaceableBaselines {public float3 diffuse; public float3 glossy;};
// Providers return a correction to their own exact shaded baseline.
public float3 compose_indirect(float3 shaded, float3 diffuse_baseline, float visibility, float3 glossy_correction, float3 diffuse_response, float3 incident) {
    return shaded-diffuse_baseline*(1.-visibility)+glossy_correction+diffuse_response*incident;
}
