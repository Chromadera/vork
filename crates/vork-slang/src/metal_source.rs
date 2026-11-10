//! Reject Slang's known unbound acceleration-structure lowering failure before
//! handing source to Metal. This also covers authored graphs outside our corpus.
use anyhow::{Result, bail};

pub fn validate(source: &str) -> Result<()> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" { i += 1; }
                i = (i + 2).min(bytes.len());
            }
            b'"' | b'\'' => {
                let quote = bytes[i]; i += 1;
                while i < bytes.len() {
                    let c = bytes[i]; i += 1;
                    if c == quote { break; }
                    if c == b'\\' { i = (i + 1).min(bytes.len()); }
                }
            }
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            }
            c if c.is_ascii_whitespace() => { i += 1; }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i; i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
                tokens.push(&source[start..i]);
            }
            _ => {
                // Emitted MSL punctuation is ASCII; skip any UTF-8 codepoint in
                // diagnostics/comments rather than slicing through its bytes.
                let end = i + source[i..].chars().next().unwrap().len_utf8();
                tokens.push(&source[i..end]); i = end;
            }
        }
    }
    let (mut braces, mut parens) = (0usize, 0usize);
    for (at, token) in tokens.iter().enumerate() {
        match *token {
            "{" => braces += 1,
            "}" => braces = braces.saturating_sub(1),
            "(" => parens += 1,
            ")" => parens = parens.saturating_sub(1),
            "acceleration_structure" if braces == 0 && parens == 0 && tokens.get(at + 1) == Some(&"<") => {
                let mut angle = 1usize;
                let mut end = at + 2;
                while end < tokens.len() && angle > 0 {
                    match tokens[end] { "<" => angle += 1, ">" => angle -= 1, _ => {} }
                    end += 1;
                }
                while matches!(tokens.get(end).copied(), Some("*" | "&" | "const" | "volatile" | "thread" | "device" | "constant")) { end += 1; }
                if let (Some(name), Some(next)) = (tokens.get(end), tokens.get(end + 1)) {
                    if matches!(*next, ";" | "=" | "[" | ",") {
                        bail!("Metal shader compilation refused: Slang emitted an unbound file-scope acceleration structure `{name}`. This ray-query helper graph is not supported by the pinned Slang Metal compiler; simplify nested trace/coverage calls or render on Vulkan.");
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const AS: &str = "metal::raytracing::acceleration_structure<metal::raytracing::instancing>";

    #[test]
    fn distinguishes_resource_globals_from_context_fields_and_parameters() {
        let good = format!("#include <metal_stdlib>\nstruct KernelContext {{ {AS} ray_scene_0; }}; void helper({AS} ray_scene_0); void entry({AS} scene) {{}} // {{\n");
        validate(&good).unwrap();
        let bad = format!("{good}\n{AS} ray_scene_0;");
        let message = validate(&bad).unwrap_err().to_string();
        assert!(message.contains("unbound file-scope acceleration structure `ray_scene_0`"));
        assert!(message.contains("Vulkan"));
    }

    #[test]
    fn handles_multiline_declarations_comments_and_return_types() {
        validate(&format!("{AS} result(); /* {{ */ const char* s=\"{{}}\";")).unwrap();
        assert!(validate(&format!("{AS}\n/* comment */ illegal;")).is_err());
    }
}
