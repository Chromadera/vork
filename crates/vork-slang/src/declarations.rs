//! A declaration view of the generated module used by compiler workspaces.
//! Bodies, internal struct members and `vork_*` compiler plumbing (public only
//! so generated stage wrappers and bundled libraries can call it) are
//! implementation details, not help text.
/// Whether a public declaration header names reserved compiler plumbing.
fn reserved(header: &str) -> bool {
    header.split('(').next().and_then(|h| h.split_whitespace().last()).is_some_and(|name| name.starts_with("vork_") || name.starts_with("__"))
}
pub(crate) fn public(source: &str) -> String {
    let mut result = String::new();
    // Member types and names of each printed struct: a struct repeating another's fields says so.
    let mut printed: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let rest = &source[cursor..];
        if rest.starts_with("//") {
            cursor += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("public ") {
            let end = rest.find(['{', ';']).expect("generated public declaration");
            // `public` is implied: help lists only the public surface.
            let full = rest[..end].split_whitespace().collect::<Vec<_>>().join(" ");
            let header = compact(full.strip_prefix("public ").unwrap_or(&full));
            if reserved(&full) || full.contains("MediumContext") || full.contains("MediumCoefficients") {
                cursor += if rest.as_bytes()[end] == b';' { end + 1 } else { closing_brace(rest, end) + 1 };
            } else if rest.as_bytes()[end] == b';' {
                result.push_str(&format!("{header};\n"));
                cursor += end + 1;
            } else {
                let close = closing_brace(rest, end);
                if header.starts_with("struct Attribute<T>") {
                    result.push_str("struct Attribute<T>{};\n");
                    cursor += close + 1;
                    continue;
                }
                if header.starts_with("struct ") || header.starts_with("enum ") {
                    let members: Vec<String> = rest[end+1..close].split(';').filter_map(|member| {
                        // Drop comments, `internal` members and the implied `public`.
                        let text: String = member.lines().map(|line| line.split("//").next().unwrap()).collect::<Vec<_>>().join(" ");
                        let member = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        (!member.is_empty() && !member.starts_with("internal ") && !(header.starts_with("struct EnvironmentContext") && member.split_whitespace().any(|word|word=="origin"))).then(|| member.strip_prefix("public ").unwrap_or(&member).to_owned())
                    }).collect();
                    // Type and name of each member (initializers aside) decide a repeat.
                    let names: Vec<String> = members.iter().map(|m| m.split('=').next().unwrap().trim().to_owned()).collect();
                    if let Some((same, _, previous)) = printed.iter().find(|(_, other, _)| !names.is_empty() && *other == names) {
                        let differences: Vec<String> = members.iter().zip(previous).filter(|(a, b)| compact(a) != compact(b)).map(|(a, _)| a.clone()).collect();
                        let note = if differences.is_empty() { "" } else { ", except:" };
                        result.push_str(&format!("{header}{{/* {same} fields{note} */"));
                        result.push_str(&fields(&differences).join(" "));
                        result.push_str("};\n");
                    } else {
                        // Keep lines readable; adjacent fields of one type share that type.
                        let mut line = format!("{header}{{");
                        let declarations = if header.starts_with("struct ") { fields(&members) } else {
                            members.iter().map(|m| format!("{};", compact(m))).collect()
                        };
                        for member in declarations {
                            if line.len() + member.len() > 100 { result.push_str(&line); result.push('\n'); line.clear(); }
                            else if !line.ends_with('{') { line.push(' '); }
                            line.push_str(&member);
                        }
                        result.push_str(&line);
                        result.push_str("};\n");
                    }
                    let name = header.split_whitespace().nth(1).unwrap_or("").to_owned();
                    printed.push((name, names, members));
                } else {
                    result.push_str(&format!("{header};\n"));
                }
                cursor += close + 1;
            }
        } else if rest.starts_with('{') {
            cursor += closing_brace(rest, 0) + 1;
        } else {
            cursor += rest.chars().next().unwrap().len_utf8();
        }
    }
    result
}
// Only generated declaration headers/fields reach here: retain every token,
// removing spaces only beside punctuation that already separates tokens.
fn compact(text: &str) -> String {
    let punctuation = |c: char| "{}[]()<>,=;".contains(c);
    let mut result = String::new();
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() { space = true; continue; }
        if space && !punctuation(ch) && result.chars().last().is_some_and(|last| !punctuation(last)) {
            result.push(' ');
        }
        result.push(ch);
        space = false;
    }
    result
}
// Slang permits `float3 a,b;`: no field, initializer or array extent is elided.
fn fields(members: &[String]) -> Vec<String> {
    let mut groups: Vec<(String, String)> = Vec::new();
    for member in members {
        let (ty, field) = member.split_once(' ').expect("generated struct field");
        let field = compact(field);
        if let Some((_, declaration)) = groups.last_mut().filter(|(previous, declaration)| previous == ty && declaration.len() + field.len() < 90) {
            declaration.push(',');
            declaration.push_str(&field);
        } else {
            groups.push((ty.to_owned(), format!("{ty} {field}")));
        }
    }
    groups.into_iter().map(|(_, declaration)| format!("{declaration};")).collect()
}
fn closing_brace(source: &str, open: usize) -> usize {
    let mut depth=0;
    let mut quoted=false;
    let mut escaped=false;
    for (at,ch) in source[open..].char_indices() {
        if escaped { escaped=false; continue; }
        if quoted && ch=='\\' { escaped=true; continue; }
        if ch=='"' { quoted=!quoted; continue; }
        if quoted { continue; }
        if ch=='{' { depth+=1; }
        if ch=='}' { depth-=1; if depth==0 { return open+at; } }
    }
    panic!("unclosed generated declaration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_surface_keeps_fields_defaults_and_signatures() {
        let source = "public struct A { public float3 a = float3(1, 2, 3); public float3 b; internal uint slot; public float uv[2]; };
public struct B { public float3 a = float3(1,2,3); public float3 b = 0; public float uv[2]; };
public float3 sample(A a, float2 uv) { return a.a; }
public uint vork_slot(A a) { return a.slot; }
public uint __hidden() { return 0; }
public struct Handle { internal uint slot; };";
        assert_eq!(public(source), "struct A{float3 a=float3(1,2,3),b; float uv[2];};\nstruct B{/* A fields, except: */float3 b=0;};\nfloat3 sample(A a,float2 uv);\nstruct Handle{};\n");
    }

    #[test]
    fn attribute_handle_hides_reserved_constructor() {
        assert_eq!(public("public struct Attribute<T> { internal uint address; public __init() { address=0; } };"), "struct Attribute<T>{};\n");
    }
}

#[cfg(test)]
mod launch_help_tests {
    #[test]
    fn transport_implementation_types_are_not_advertised() {
        let source="public struct MediumContext { public float3 position; }; public struct MediumCoefficients { public float3 absorption; }; public struct EnvironmentContext { public float3 origin; public float3 direction; };";
        let help=super::public(source);
        assert!(!help.contains("Medium"));
        assert!(!help.contains("origin"));
        assert!(help.contains("direction"));
    }
}
