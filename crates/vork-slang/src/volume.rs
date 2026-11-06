//! The medium stage's physical coefficients (linear Rec.709 scene space, per metre).
use super::*;
fn transport_fields() -> Vec<Field> {
    let names = ["absorption", "scattering", "emission", "anisotropy"];
    names
        .iter()
        .map(|name| {
            Field::new(
                name,
                if *name == "anisotropy" {
                    Type::Float
                } else {
                    Type::Vector(3)
                },
                Some("0"),
            )
        })
        .collect()
}
fn slang_struct(name: &str, fields: &[Field]) -> String {
    let mut text = format!("public struct {name} {{\n");
    for field in fields {
        writeln!(
            text,
            "public {} {}={};",
            field.ty.slang(),
            field.name,
            field.default.as_deref().unwrap_or("0")
        )
        .unwrap();
    }
    text.push_str("};\n");
    text
}
fn wgsl_struct(name: &str, fields: &[Field]) -> String {
    let members: Vec<_> = fields.iter().map(Field::wgsl).collect();
    format!("struct {name} {{ {} }}\n", members.join(","))
}
pub(crate) fn module() -> String {
    slang_struct("MediumCoefficients", &transport_fields())
}
pub fn transport_wgsl() -> String {
    wgsl_struct("MediumCoefficients", &transport_fields())
}
