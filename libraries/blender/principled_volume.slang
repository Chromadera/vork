// Blender 5.2.1 Principled Volume as a medium program: its sockets, defaults and
// limits as Inputs, converted with Blender's formulas (blender.volume).
import vork;
import blender.volume;
public struct Inputs {
    [InputRange(0.0, 3.402823466e38, true, true)] float3 color = float3(0.5);
    [InputRange(0.0, 3.402823466e38, true, true)] float density = 1.0;
    [InputRange(-1.0, 1.0, true, true)] float anisotropy = 0.0;
    [InputRange(0.0, 3.402823466e38, true, true)] float3 absorption_color = float3(0.0);
    [InputRange(0.0, 3.402823466e38, true, true)] float emission_strength = 0.0;
    [InputRange(0.0, 3.402823466e38, true, true)] float3 emission_color = float3(1.0);
    [InputRange(0.0, 1.0, true, true)] float blackbody_intensity = 0.0;
    [InputRange(0.0, 3.402823466e38, true, true)] float3 blackbody_tint = float3(1.0);
    [InputRange(0.0, 3.402823466e38, true, true)] float temperature = 1000.0;
};
MediumCoefficients medium(MediumContext ctx, Inputs inputs) {
    MediumPoint point;
    point.color=inputs.color; point.density=inputs.density;
    point.anisotropy=inputs.anisotropy; point.absorption_color=inputs.absorption_color;
    point.emission_strength=inputs.emission_strength; point.emission_color=inputs.emission_color;
    point.blackbody_intensity=inputs.blackbody_intensity; point.blackbody_tint=inputs.blackbody_tint;
    point.temperature=inputs.temperature;
    return principled_volume(point);
}
