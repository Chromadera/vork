#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
import vork_prelude;
import vork_constants;
import vork_medium_types;
import vork_light_falloff;
internal extern Light medium_light(uint index);
internal extern float3 medium_visibility(float3 position,uint index);
#if VORK_VOLUME_SHADOWS
internal extern float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint);
#endif
// Keep the full shadow/extinction service at one native call boundary. It is
// linked by both camera transport and secondary segments, including exterior rays.
[noinline]
public export float3 medium_direct_visibility(float3 position,uint index) {
    var value=medium_visibility(position,index);
#if VORK_VOLUME_SHADOWS
    if(any(value>float3(0.))) {
        let light=medium_light(index);
        let falloff=light_falloff(position,light.position,light.direction,light.radiance.w,light.projection.y,light.projection.z>0.);
        let endpoint=light.position.w==0. ? medium_infinity() : medium_emitter(length(light.position.xyz-position));
        value*=medium_transmittance(position+falloff.axis*REG_POSITIVE_EPSILON,falloff.axis,endpoint);
    }
#endif
    return value;
}
