//! Generates the OpenPBR parameter table from the checked-in specification copy.
use std::{env, fmt::Write, fs, path::Path};

fn main() {
    // The private corpus and its test modules are intentionally absent from public exports.
    println!("cargo:rustc-check-cfg=cfg(vork_private_tests)");
    println!("cargo:rerun-if-changed=build.rs");
    if std::path::Path::new("../../scenes").is_dir() {
        println!("cargo:rerun-if-changed=../../scenes");
        println!("cargo:rustc-cfg=vork_private_tests");
    }
    let source = "openpbr-params.md";
    println!("cargo:rerun-if-changed={source}");
    let text = fs::read_to_string(source).expect("read openpbr-params.md");
    let mut out = String::from("&[\n");
    // The renderer's evaluated surface: the same parameters, same names, as WGSL.
    let mut fields = String::new();
    let mut defaults = Vec::new();
    // The same closure as a Slang module, for the Slang raster passes.
    let mut slang_fields = String::new();
    let mut slang_defaults = Vec::new();
    for line in text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // | `identifier` | type | range | norm | default | unit | meaning |
        let Some(name) = cells
            .get(1)
            .and_then(|c| c.strip_prefix('`')?.strip_suffix('`'))
        else {
            continue;
        };
        let (kind, range, default) = (cells[2], cells[3], cells[5]);
        let unit = cells[6];
        let meaning = cells[7];
        let default_text = default;
        let closure = cells.get(8).copied() != Some("Input only");
        let supported = cells.get(8).copied().unwrap_or("") != "Closure only";
        let kind = match kind {
            "float" => "Float",
            "color3" => "Color3",
            "boolean" => "Bool",
            "vector3" => "Vector3",
            other => panic!("{name}: unknown OpenPBR type {other}"),
        };
        let numbers = |s: &str| -> Vec<f32> {
            s.trim_matches(|c| "()[]".contains(c))
                .split(',')
                .map(|x| match x.trim() {
                    "∞" => f32::INFINITY,
                    x => x.parse().unwrap_or_else(|_| panic!("{name}: number {x}")),
                })
                .collect()
        };
        let default = match kind {
            "Bool" => vec![f32::from(default == "true")],
            "Vector3" => vec![],
            _ => numbers(default),
        };
        let (min, max, open_min, open_max) = if matches!(kind, "Bool" | "Vector3") {
            (0., 1., false, false)
        } else {
            let range = range.trim_end_matches('³');
            let bounds = numbers(range);
            (
                bounds[0],
                bounds[1],
                range.starts_with('('),
                range.ends_with(')'),
            )
        };
        let wgsl = |v: &[f32]| {
            v.iter()
                .map(|x| format!("{x:?}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let (ty, value) = match kind {
            "Float" => ("f32", wgsl(&default)),
            "Color3" => ("vec3<f32>", format!("vec3({})", wgsl(&default))),
            "Bool" => ("bool", (default[0] != 0.).to_string()),
            _ => (
                "vec3<f32>",
                if name.ends_with("tangent") {
                    "tangent"
                } else {
                    "normal"
                }
                .into(),
            ),
        };
        if closure {
            writeln!(fields, "    {name}:{ty},").unwrap();
            let slang_ty = match ty {
                "f32" => "float",
                "bool" => "bool",
                _ => "float3",
            };
            writeln!(slang_fields, "    public {slang_ty} {name};").unwrap();
            slang_defaults.push(value.replace("vec3(", "float3("));
            defaults.push(value);
        }
        writeln!(
            out,
            "    OpenPbrParam {{ name: {name:?}, kind: OpenPbrKind::{kind}, default: &{default:?}, unit: {unit:?}, meaning: {meaning:?}, default_text: {default_text:?}, supported: {supported}, closure: {closure}, \
             min: {}, max: {}, open_min: {open_min}, open_max: {open_max} }},",
            f(min),
            f(max),
        )
        .unwrap();
    }
    out.push(']');
    let count = out.matches("OpenPbrParam {").count();
    assert_eq!(
        count, 37,
        "36 OpenPBR parameters plus std cutoff, table has {count}"
    );
    let dir = Path::new(&env::var("OUT_DIR").unwrap()).to_path_buf();
    fs::write(dir.join("openpbr.rs"), out).unwrap();
    let closure = dir.join("closure.wgsl");
    let wgsl = format!(
        "// Generated from openpbr-params.md: OpenPBR Surface 1.1.1.\n\
         struct Closure {{\n{fields}}};\n\
         // OpenPBR defaults; normals and tangents default to the unperturbed frame.\n\
         fn closure_defaults(normal:vec3<f32>,tangent:vec3<f32>)->Closure {{\n    return Closure({});\n}}\n",
        defaults.join(",")
    );
    fs::write(&closure, wgsl).unwrap();
    let slang = format!(
        "// Generated from openpbr-params.md: OpenPBR Surface 1.1.1.\n\
         module closure;\n\
         public struct Closure {{\n{slang_fields}}};\n\
         // OpenPBR defaults; normals and tangents default to the unperturbed frame.\n\
         public Closure closure_defaults(float3 normal, float3 tangent) {{\n    return Closure({});\n}}\n",
        slang_defaults.join(",")
    );
    fs::write(dir.join("closure.slang"), slang).unwrap();
    // Dependents (the renderer's shader build) read it through DEP_VORK_OPENPBR_CLOSURE.
    println!("cargo:closure={}", closure.display());
}

fn f(x: f32) -> String {
    if x.is_infinite() {
        "f32::INFINITY".into()
    } else {
        format!("{x:?}")
    }
}
