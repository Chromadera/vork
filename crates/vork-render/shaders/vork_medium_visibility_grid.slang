#ifndef VORK_MEDIUM_SCATTERING_FREE
#define VORK_MEDIUM_SCATTERING_FREE 0
#endif
import vork_prelude;
import vork_constants;
internal extern Frame medium_frame();
internal extern Frame medium_visibility_frame();
internal extern Light medium_light(uint index);
internal extern uint2 medium_size();
internal extern float4 medium_visibility_read(uint index);
internal extern uint medium_visibility_sample();
public float medium_visibility_scale() {return max(medium_visibility_frame().params.y*.5,REG_POSITIVE_EPSILON);}
public float3 medium_visibility_position(float3 cell) {
    // Endpoint strata must stay inside the inverse map's finite domain.
    // Subtracting the positive denominator floor from one rounds back to one.
    let limit=screen_next_float(1.,false);
    let q=clamp((cell+.5)*(2./float(REG_MEDIUM_VISIBILITY_SIDE))-1.,-limit,limit);
    return medium_visibility_frame().eye.xyz+medium_visibility_scale()*q/(1.-abs(q));
}
public uint medium_visibility_slot(uint light) {
    return medium_light(light).shadow_offsets[REG_LIGHT_VOLUME_SLOT_WORD];
}
public uint medium_visibility_address(uint3 cell,uint slot) {
    let s=medium_size();let f=medium_frame();let side=REG_MEDIUM_VISIBILITY_SIDE;
    let index=(cell.z*side+cell.y)*side+cell.x;
    return s.x*s.y*(max(f.sampling.w,1u)+1u)
        +slot*side*side*side+index;
}
public export float3 medium_cached_visibility(float3 position,uint light) {
#if VORK_MEDIUM_SCATTERING_FREE
    return float3(0.);
#else
    let slot=medium_visibility_slot(light);
    let v=position-medium_visibility_frame().eye.xyz;
    let q=v/(medium_visibility_scale()+abs(v));
    let coord=(q+1.)*(.5*float(REG_MEDIUM_VISIBILITY_SIDE))-.5;
    let base=floor(coord);let blend=frac(coord);var sum=float3(0.);
    for(uint z=0u;z<2u;z++) {for(uint y=0u;y<2u;y++) {for(uint x=0u;x<2u;x++) {
        let cell=uint3(clamp(base+float3(x,y,z),float3(0.),float3(REG_MEDIUM_VISIBILITY_SIDE-1u)));
        let weight=select(x==1u,blend.x,1.-blend.x)*select(y==1u,blend.y,1.-blend.y)*select(z==1u,blend.z,1.-blend.z);
        sum+=weight*medium_visibility_read(medium_visibility_address(cell,slot)).rgb;
    }}}
    return sum;
#endif
}
