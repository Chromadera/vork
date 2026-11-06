//! Curves-only hair closure. Packed independently of the OpenPBR closure.
use super::*;
pub const HAIR_VEC4S: usize = 3;
pub fn hair_fields() -> Vec<Field> {
    let mut fields = vec![Field::new("absorption_coefficient", Type::Vector(3), None)];
    for name in ["roughness", "radial_roughness", "coat", "ior", "offset"] {
        fields.push(Field::new(name, Type::Float, None));
    }
    fields.push(Field::new("color", Type::Vector(3), None));
    // Closure transport control, rather than an artist socket default.
    fields.push(Field::new("scattering", Type::Bool, Some("true")));
    fields
}
pub(crate) fn module() -> String {
    let mut s = String::from("public struct Hair {\n");
    for f in hair_fields() {
        if let Some(default) = &f.default {
            writeln!(s, "public {} = {default};", f.slang()).unwrap();
        } else {
            writeln!(s, "public {};", f.slang()).unwrap();
        }
    }
    s.push_str("};\n");
    s
}
