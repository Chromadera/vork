import vork_constants;
// Blender radiometric lights and glTF punctual cone/range semantics, shared by surfaces and media.
struct LightFalloff { float3 axis; float range_cone; float punctual; };
LightFalloff light_falloff(float3 position, float4 light_position, float4 direction, float inner, float light_range, bool gltf) {
    var axis=normalize(-direction.xyz);var distance2=1.;var window=1.;var weight=1.;
    if (light_position.w>0.) {
        let delta=light_position.xyz-position;distance2=max(dot(delta,delta),REG_LIGHT_MIN_DISTANCE_SQUARED);
        axis=delta*rsqrt(distance2);weight=1./(4.*REG_PI*distance2);
        if (light_range>0.) {let ratio=distance2/(light_range*light_range);window=max(1.-ratio*ratio,0.);}
    }
    var cone=1.;
    if (light_position.w==2.) {
        let cosine=dot(-axis,normalize(direction.xyz));
        cone=select(cosine>=direction.w,1.,0.);
        if (inner>direction.w) {
            if (gltf) {let t=clamp((cosine-direction.w)/(inner-direction.w),0.,1.);cone=t*t;}
            else {cone=smoothstep(direction.w,inner,cosine);}
        }
    }
    let range_cone=window*cone;
    // Media approximate finite rectangles by their projected centre; surfaces use the exact LTC integral.
    if (light_position.w==3.) {weight*=4.*max(dot(direction.xyz,-axis),0.);}
    return LightFalloff(axis,range_cone,weight*range_cone);
}

