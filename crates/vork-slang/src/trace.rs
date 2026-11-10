//! Geometric query ABI. WGSL is an analysis representation, never a provider.
/// Conservative selection of the immutable geometry read by a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct QueryFilter {
    pub mask: u8,
    /// None selects every public object identity.
    pub object: Option<u32>,
}

pub(crate) fn module(native: bool) -> String {
    // Queries are opaque resource operations: reachable calls retain work
    // accounting and diagnostics even when the caller ignores hit data.
    if native {
        return "__exported import program_trace;\ninternal extern TraceHit native_geometric_trace(float3 origin,float3 direction,float minimum,float maximum,uint mask,uint object);\npublic TraceHit trace(float3 origin,float3 direction,float minimum,float maximum,uint mask=255u,uint object=0xffffffffu) {return native_geometric_trace(origin,direction,minimum,maximum,mask,object);}\n".into();
    }
    let mut source = include_str!("trace_types.slang").to_owned();
    source.push_str(r#"
float4 trace_field(float3 origin,float3 direction,float minimum,float maximum,uint mask,uint object,uint field) {
    __target_switch { case wgsl: __intrinsic_asm "vork_trace_field($0,$1,$2,$3,$4,$5,$6)"; }
}
public TraceHit trace(float3 origin,float3 direction,float minimum,float maximum,uint mask=255u,uint object=0xffffffffu) {
    TraceHit h;
    let a=trace_field(origin,direction,minimum,maximum,mask,object,0u);
    let b=trace_field(origin,direction,minimum,maximum,mask,object,1u);
    let c=trace_field(origin,direction,minimum,maximum,mask,object,2u);
    let d=trace_field(origin,direction,minimum,maximum,mask,object,3u);
    let e=trace_field(origin,direction,minimum,maximum,mask,object,4u);
    let f=trace_field(origin,direction,minimum,maximum,mask,object,5u);
    h.hit=a.x!=0.;h.distance=a.y;h.front_facing=a.z!=0.;
    h.position=b.xyz;h.geometric_normal=c.xyz;h.shading_normal=d.xyz;
    h.object_id=asuint(e.x);h.instance_index=asuint(e.y);h.primitive_id=asuint(e.z);
    h.barycentrics=f.xyz;h.uv=float2(b.w,c.w);return h;
}
"#);
    source
}
pub(crate) fn validation_stubs() -> &'static str {
    "fn vork_trace_field(o:vec3<f32>,d:vec3<f32>,lo:f32,hi:f32,mask:u32,object:u32,field:u32)->vec4<f32>{return vec4(o+d,lo+hi+f32(mask+object+field));}\n"
}
