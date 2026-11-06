//! Typed, install-resolved vertex attribute handles. Packing remains kernel-private.
pub(crate) fn module(native: bool) -> String {
    let read = if native {
        "return material_attribute(slot, differential);".to_owned()
    } else {
        "__target_switch { case wgsl: __intrinsic_asm \"vork_attribute($0, $1)\"; }".to_owned()
    };
    let mut source = format!(
        "public struct Attribute<T> {{ internal uint address; internal T type_witness; public __init() {{ address=0xffffffffu; }} }};\npublic Attribute<T> vork_attribute_handle<T>(uint address) {{ Attribute<T> a; a.address=address; return a; }};\npublic uint vork_slot<T>(Attribute<T> a) {{ return a.address; }}\npublic struct AttributeSample<T> {{ public T value; public T dx; public T dy; }};\nfloat4 attribute_raw(uint slot,uint differential) {{ {read} }}\n"
    );
    source.push_str("float4 attribute_read(uint address,uint differential) { uint descriptor=vork_input_word(address);float4 v=attribute_raw(descriptor&255u,differential);uint channel=descriptor>>8u;if(channel<4u) v=float4(v[channel]);return v*float4(asfloat(vork_input_word(address+1u)),asfloat(vork_input_word(address+2u)),asfloat(vork_input_word(address+3u)),asfloat(vork_input_word(address+4u))); }\n");
    source.push_str("public struct DisplacementContext { public float3 POSITION; public float3 NORMAL; public float time; };\n");
    for (ty, swizzle) in [
        ("float", ".x"),
        ("float2", ".xy"),
        ("float3", ".xyz"),
        ("float4", ""),
    ] {
        source.push_str(&format!("public AttributeSample<{ty}> read_attribute(Attribute<{ty}> a, ShadingContext ctx) {{ AttributeSample<{ty}> s;s.value=attribute_read(a.address,0u){swizzle};s.dx=attribute_read(a.address,1u){swizzle};s.dy=attribute_read(a.address,2u){swizzle};return s; }}\n"));
        source.push_str(&format!("public {ty} read_attribute(Attribute<{ty}> a, DisplacementContext ctx) {{ return attribute_read(a.address,0u){swizzle}; }}\n"));
    }
    source
}
