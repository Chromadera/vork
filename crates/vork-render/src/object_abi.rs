//! Packed object ABI; this table owns host, Slang and WGSL declarations.
const FIELDS: &[(&str, &str, &str, &str)] = &[
    ("model", "[[f32; 4]; 3]", "float4 model[3]", "mat3x4<f32>"),
    ("previous_model", "[[f32; 4]; 3]", "float4 previous_model[3]", "mat3x4<f32>"),
    ("next_model", "[[f32; 4]; 3]", "float4 next_model[3]", "mat3x4<f32>"),
    ("opacity", "[f32; 2]", "float2 opacity", "vec2<f32>"),
    ("material_index", "u32", "uint material_index", "u32"),
    ("reserved", "u32", "uint reserved", "u32"),
    ("metadata", "[u32; 2]", "uint2 metadata", "vec2<u32>"),
    ("padding", "[u32; 2]", "uint2 padding", "vec2<u32>"),
];
pub fn rust() -> String {
    let fields: String = FIELDS.iter().map(|(name, ty, _, _)| format!("{name}: {ty},\n")).collect();
    format!("#[repr(C)]\n#[derive(Clone, Copy, Pod, Zeroable)]\nstruct GpuObject {{{fields}}}\n")
}
pub fn slang() -> String {
    let fields: String = FIELDS.iter().map(|(_, _, field, _)| format!("{field};")).collect();
    format!("struct Object {{{fields}}};\n")
}
pub fn wgsl(name: &str) -> String {
    let fields: String = FIELDS.iter().map(|(name, _, _, ty)| format!("{name}:{ty},")).collect();
    format!("struct {name} {{{fields}}};\n")
}
