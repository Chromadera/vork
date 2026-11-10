//! Value boundaries for generated native material functions.
//!
//! Slang lowers aggregate input parameters to borrowed pointers. Its SPIR-V
//! address-space specialization can then clone an outlined material graph for
//! different callers. Pass packed context vectors by value and return the packed
//! outputs together; the inline bridge preserves the renderer's array ABI.
//! This only consumes compiler-generated declarations, never authored Slang.

const MARKER: &str = "#ifdef VORK_PATH_SLANG_OUTLINE\n[noinline]\n#endif\n";

pub fn outline(source: &str) -> String {
    let Some((prefix, rest)) = source.split_once(MARKER) else {
        return source.into();
    };
    let (declaration, tail) = rest
        .split_once('{')
        .expect("generated material function body");
    let (head, parameters) = declaration
        .trim()
        .strip_suffix(')')
        .unwrap()
        .split_once('(')
        .unwrap();
    let mut head = head.split_whitespace();
    assert_eq!(head.next(), Some("public"));
    let returns = head.next().unwrap();
    assert!(matches!(returns, "void" | "bool"));
    let name = head.next().unwrap();
    assert!(head.next().is_none());
    let mut depth = 1;
    let end = tail
        .char_indices()
        .find_map(|(i, c)| {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(i)
        })
        .expect("generated material function closing brace");
    let body = &tail[..end];
    let suffix = &tail[end + 1..];
    let mut core = Vec::new();
    let mut args = Vec::new();
    let mut locals = String::new();
    let mut fields = String::new();
    let mut pack = String::new();
    let mut unpack = String::new();
    for parameter in parameters.split(',').map(str::trim) {
        if let Some(output) = parameter.strip_prefix("out ") {
            let (_, field) = output.split_once(' ').unwrap();
            let field = field.split('[').next().unwrap();
            locals.push_str(&format!("{output};\n"));
            fields.push_str(&format!("{output};\n"));
            pack.push_str(&format!("value.{field}={field};\n"));
            unpack.push_str(&format!("{field}=value.{field};\n"));
        } else if let Some(array) = parameter
            .strip_prefix("float4 ")
            .and_then(|s| s.strip_suffix(']'))
        {
            let (array, count) = array.split_once('[').unwrap();
            let count: usize = count.parse().unwrap();
            let mut values = Vec::new();
            for i in 0..count {
                core.push(format!("float4 {array}{i}"));
                args.push(format!("{array}[{i}]"));
                values.push(format!("{array}{i}"));
            }
            locals.push_str(&format!(
                "float4 {array}[{count}]={{{}}};\n",
                values.join(",")
            ));
        } else {
            assert!(
                !parameter.contains('['),
                "unexpected generated aggregate parameter"
            );
            core.push(parameter.into());
            args.push(parameter.split_whitespace().last().unwrap().into());
        }
    }
    let result = format!("{name}_result");
    let ret = |value: &str| format!("{{{result} value;\n{pack}{value}return value;}}");
    let body = if returns == "bool" {
        fields.push_str("bool ok;\n");
        body.replace("return true;", &ret("value.ok=true;\n"))
            .replace("return false;", &ret("value.ok=false;\n"))
    } else {
        body.replace("return;", &ret("")) + &ret("")
    };
    let return_value = if returns == "bool" {
        "return value.ok;"
    } else {
        ""
    };
    // Keep the original function name on the core: Blend scratch localization
    // identifies the owned replay boundary by this generated name.
    format!(
        "{prefix}#ifdef VORK_PATH_SLANG_OUTLINE\nstruct {result} {{{fields}}};\n[noinline]\n{result} {name}({}) {{{locals}{body}}}\n{declaration}{{{result} value={name}({});\n{unpack}{return_value}}}\n#else\n{declaration}{{{}}}\n#endif\n{}",
        core.join(","),
        args.join(","),
        &tail[..end],
        outline(suffix)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compiler;

    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn value_boundary_compiles_on_both_native_targets() -> anyhow::Result<()> {
        let source = outline(&format!(
            r#"
{MARKER}public bool lobe_dispatch(uint program,float4 a[2],uint base,out float4 r[3],out float2 lobes) {{
 r={{}};lobes=1.;switch(program){{case 1u:r[0]=a[0];return true;default:return false;}}
}}
{MARKER}public void vork_surface_000000000001(float4 a[2], uint base, float4 light[2], out float4 r[3]) {{r={{a[0],a[1],light[base%2]}};}}
RWStructuredBuffer<float4> output;
[numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID) {{
 float4 a[2]={{output[0],output[1]}};float4 r[3];float2 lobes;
 bool found=lobe_dispatch(id.x,a,id.y,r,lobes);
 output[2]=r[0]+float4(lobes,float(found),0.);
 vork_surface_000000000001(a,id.z,a,r);output[3]=r[2];
}}
"#
        ));
        for target in ["spirv", "metal"] {
            for outlined in [false, true] {
                let mut args = vec!["-target", target, "-O2"];
                if outlined {
                    args.push("-DVORK_PATH_SLANG_OUTLINE");
                }
                let code = Compiler::discover().compile_modules(
                    &[("main", source.as_str())],
                    "main",
                    &[("main", "compute")],
                    &args,
                )?;
                assert!(!code.is_empty());
            }
        }
        Ok(())
    }
}
