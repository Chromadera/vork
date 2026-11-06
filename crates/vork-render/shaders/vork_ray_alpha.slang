import vork_constants;
import vork_material_table;
import vork_shadow_coverage;
// The consuming pass supplies its retained hit/alpha table. Keeping the resource
// explicit lets unused ray variants eliminate the binding on every backend.
public struct RayIntersection { public uint instance_index; public uint instance_custom_data; public uint primitive_index; public float2 barycentrics; };
import frame;
public uint2 ray_fields(StructuredBuffer<uint> words) {return frame.field_geometry.xy;}
// Alpha-tested ray candidates (rt.rs alpha table): two words per TLAS instance,
// the first corner word (REG_RAY_NO_ALPHA when opaque) and the material record,
// then per corner TEXCOORD_0, TEXCOORD_1 and vertex-colour alpha. Coverage at the
// hit is material_alpha, the raster caster rule, at the textures' base level.
public float4 ray_corner(StructuredBuffer<uint> words, uint corner) {
    return float4(asfloat(words[corner]),asfloat(words[corner+1u]),asfloat(words[corner+2u]),asfloat(words[corner+3u]));
}
// Surface hits and shadows consume the same evaluated scalar coverage.
public float ray_candidate_opacity(StructuredBuffer<uint> words, RayIntersection hit, uint r) {
    let start=words[hit.instance_index*REG_RAY_SLOT_WORDS];
    if (start==REG_RAY_NO_ALPHA) {return 1.;}
    let corner=start+hit.primitive_index*15u;
    let b=float3(1.-hit.barycentrics.x-hit.barycentrics.y,hit.barycentrics);
    let uv=ray_corner(words,corner)*b.x+ray_corner(words,corner+5u)*b.y+ray_corner(words,corner+10u)*b.z;
    let alpha=dot(b,float3(asfloat(words[corner+4u]),asfloat(words[corner+9u]),asfloat(words[corner+14u])));
    let m=MaterialUv(uv.xy,uv.zw,float2(0.),float2(0.),float2(0.),float2(0.),float3(0.));
    return candidate_scalar_opacity(r,material_alpha(r,m,alpha));
}
