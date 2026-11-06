// Blender 5.2.1 AgX sRGB and looks, using the original core half-float samples.
// See agx-manifest.json and scripts/color/bake-agx-looks.py.
import vork;
// Blender's look names: None, Very High Contrast, ..., Very Low Contrast.
public enum AgxLook {
    None = 0, Punchy = 1, Greyscale = 2, Very_High_Contrast = 3,
    High_Contrast = 4, Medium_High_Contrast = 5, Base_Contrast = 6,
    Medium_Low_Contrast = 7, Low_Contrast = 8, Very_Low_Contrast = 9
};
import std.display;
public struct Inputs {
    float exposure = 0.;
    [InputRange(0., 3.402823466e38, false, true)] float gamma = 1.;
    [InputRange(0., 3.402823466e38, true, true)] float dither_intensity = 1.;

    AgxLook look = AgxLook::None;
    // Unbound: the library's bake above.
    Image lut;
};
import blender.display_lut;
public float3 display(DisplayContext ctx, Inputs inputs) {
    ctx.color *= exp2(inputs.exposure);
    return finish_display(display_lut(ctx,inputs.lut,int(inputs.look),10.),ctx,inputs.gamma,inputs.dither_intensity);
}
