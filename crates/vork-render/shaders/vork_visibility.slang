import vork_constants;
internal extern bool links_intersection_words(uint a,uint b);
public bool links_intersect(uint a,uint b) {return links_intersection_words(a,b);}
// Keep surface ray visibility in spare bits of the existing half-float payload.
// Two RGB5/roughness fields occupy 0..1023; a second 1024-wide range plus sign
// remains exact in f16 (all integers through +/-2048). Default masks preserve it.
public float surface_visibility_payload(float encoded) {
    return select(encoded<0.,-encoded-1.,encoded);
}
public float surface_reflection_payload(float encoded) {
    return surface_visibility_payload(encoded)%REG_VISIBILITY_DIFFUSE_OFFSET;
}
// The resolved ambient target uses its payload for source-colour composition state.
public bool surface_composited(float encoded) {return surface_reflection_payload(encoded)!=0.;}
public bool surface_visible_specular(float encoded) {return encoded>=0.;}
public bool surface_visible_diffuse(float encoded) {return surface_visibility_payload(encoded)<REG_VISIBILITY_DIFFUSE_OFFSET;}
public float surface_encode_visibility(float payload, uint mask) {
    let value=payload+select((mask&REG_RAY_DIFFUSE)!=0u,0.,REG_VISIBILITY_DIFFUSE_OFFSET);
    return select((mask&REG_RAY_SPECULAR)!=0u,value,-value-1.);
}

// -(payload+2) excludes world specular while retaining traced reflection.
// RGB5 pair <=1023 remains exactly representable in the half-float attachment.
public bool surface_world_specular(float encoded) {return encoded>=0.;}
public float surface_specular_payload(float encoded) {return select(encoded<0.,-encoded-REG_WORLD_SPECULAR_OFFSET,encoded);}

public bool light_receives(uint contribution, uint selection, uint groups) {
    return (contribution&REG_LIGHT_LINK_ALL)!=0u || links_intersect(selection,groups);
}


public float surface_reflection_roughness(float encoded) {
    return floor(surface_reflection_payload(encoded)/REG_REFLECTION_PACK_BASE)/REG_REFLECTION_PACK_LEVELS;
}
// Coverage and tracing must use the same packed control representation.
public bool surface_reflection_traceable(float2 payload) {
    return surface_specular_payload(payload.x)>0. || surface_reflection_payload(payload.y)%REG_REFLECTION_PACK_BASE>0.;
}
