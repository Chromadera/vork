import std.color;
// Blender 5.2.1 Principled Volume numeric sockets (linear Rec.709 scene space) and
// their conversion to the medium stage's physical coefficients.
import vork;
public struct MediumPoint {
    public float3 color = float3(0.5);
    public float density = 1.0;
    public float anisotropy = 0.0;
    public float3 absorption_color = float3(0.0);
    public float emission_strength = 0.0;
    public float3 emission_color = float3(1.0);
    public float blackbody_intensity = 0.0;
    public float3 blackbody_tint = float3(1.0);
    public float temperature = 1000.0;
};
static const float PV_STEFAN_BOLTZMANN = 5.670373e-8;
static const float PV_BLACKBODY_UNIT_SCALE = 1e-6;
static const float PV_PI = 3.141592653589793;
public MediumCoefficients principled_volume(MediumPoint point) {
    MediumCoefficients result;
    float density=max(point.density,0.0);
    float3 albedo=max(point.color,0.0);
    result.scattering=density*albedo;
    result.absorption=density*max(1.0-albedo,0.0)*max(1.0-sqrt(max(point.absorption_color,0.0)),0.0);
    result.emission=max(point.emission_strength,0.0)*max(point.emission_color,0.0);
    if(point.blackbody_intensity>0.0) {
        float t=max(point.temperature,0.0);
        float power=(PV_STEFAN_BOLTZMANN*PV_BLACKBODY_UNIT_SCALE/PV_PI)*lerp(1.0,t*t*t*t,point.blackbody_intensity);
        result.emission+=max(point.blackbody_tint,0.0)*power*blackbody(t);
    }
    result.anisotropy=clamp(point.anisotropy,-0.999,0.999);
    return result;
}
