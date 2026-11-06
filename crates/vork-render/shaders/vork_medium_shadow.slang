// Stable sample coordinates for medium receiver connections.
import vork_constants;
import vork_prelude;
import vork_shadow_visibility;
import vork_light_falloff;
import variant;
internal extern Frame medium_frame();
public uint2 medium_shadow_pixel(float3 position) {
    let f=medium_frame();let clip=mul(float4(position,1.),f.vp);
    return uint2(max((clip.xy/clip.w*float2(.5,-.5)+.5)*float2(65536.),float2(0.)));
}
