// Custom Hair program for testing diffuse and physical closure consumers.
import vork;
import std.hair_bsdf;
enum HairModel { Chiang, Diffuse };
struct Inputs {
    HairModel model = HairModel::Diffuse;
    float3 color = float3(0.017513,0.005763,0.002059);
    float roughness = 0.3;
    float offset = 0.034906585;
};
Hair surface(ShadingContext ctx, Inputs i) {
    var h = principled_hair(i.color,i.roughness,0.3,0.,1.55,i.offset);
    h.color=max(i.color,float3(0.));
    h.scattering = i.model == HairModel::Chiang;
    return h;
}
