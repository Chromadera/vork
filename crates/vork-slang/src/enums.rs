//! Recover enum identity erased by Slang's JSON scalar reflection. The pinned
//! compiler's AST supplies declarations and source spans, never a source regex.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputEnum {
    pub ty: String,
    pub cases: Vec<String>,
}

fn name(record: &str) -> Result<&str> {
    record
        .split("nameAndLoc : NameLoc{\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .context("Slang AST declaration name missing")
}
fn offset(record: &str) -> Result<usize> {
    record
        .split("loc : SourceLoc(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .context("Slang AST source span missing")?
        .parse()
        .context("Slang AST source span changed")
}
fn parent(record: &str) -> Option<&str> {
    record
        .lines()
        .find_map(|line| line.trim().strip_prefix("parentDecl : "))
}

pub(super) fn reflect(directory: &Path, inputs: &mut InputLayout) -> Result<()> {
    let mut enums = BTreeMap::<String, Vec<String>>::new();
    let mut fields = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "slang-ast") {
            continue;
        }
        let ast = read_compiler_text(&path)?;
        let records: BTreeMap<_, _> = ast
            .split("\n}\n")
            .filter_map(|record| record.trim().split_once(" {\n"))
            .collect();
        let Some((_, module)) = records
            .iter()
            .find(|(key, _)| key.starts_with("ModuleDecl:"))
        else {
            continue;
        };
        for (id, record) in &records {
            if id.starts_with("EnumDecl:") {
                let mut cases: Vec<_> = records
                    .iter()
                    .filter(|(key, r)| key.starts_with("EnumCaseDecl:") && parent(r) == Some(*id))
                    .map(|(_, r)| Ok((offset(r)?, name(r)?.to_owned())))
                    .collect::<Result<_>>()?;
                cases.sort_by_key(|(position, _)| *position);
                let cases: Vec<_> = cases.into_iter().map(|(_, name)| name).collect();
                if !cases.is_empty() {
                    enums.insert(name(record)?.to_owned(), cases);
                }
            }
        }
        let Some((input_id, _)) = records.iter().find(|(key, record)| {
            key.starts_with("StructDecl:") && name(record).ok() == Some("Inputs")
        }) else {
            continue;
        };
        let source_path = directory.join(format!("{}.slang", name(module)?));
        let source_path = if source_path.exists() {
            source_path
        } else {
            let location = module
                .lines()
                .find_map(|line| line.trim().strip_prefix("loc : SourceLoc("))
                .context("Slang AST module location missing")?;
            let file = location
                .split_once(") ")
                .and_then(|(_, s)| s.rsplit_once(':').map(|(file, _)| file))
                .context("Slang AST module source missing")?;
            PathBuf::from(file)
        };
        let source = fs::read_to_string(&source_path)
            .with_context(|| format!("read enum probe source {}", source_path.display()))?;
        let base = offset(module)?;
        for (id, record) in &records {
            if !id.starts_with("VarDecl:") || parent(record) != Some(*input_id) {
                continue;
            }
            let Some(expression) = record
                .split("type :")
                .nth(1)
                .and_then(|s| s.split_once('{').map(|(_, s)| s.trim()))
                .and_then(|s| s.lines().next())
                .map(|s| s.trim().trim_end_matches(','))
            else {
                continue;
            };
            let Some(expression) = records.get(expression) else {
                continue;
            };
            let start = offset(expression)?
                .checked_sub(base)
                .context("invalid Slang source span")?;
            let text = source
                .get(start..)
                .context("Slang enum source span outside module")?;
            let ty: String = text
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            fields.push((name(record)?.to_owned(), ty));
        }
    }
    for (name, ty) in fields {
        if let Some(cases) = enums.get(&ty) {
            if let Some(field) = inputs.fields.iter_mut().find(|f| f.name == name) {
                ensure!(
                    matches!(field.ty, Type::Int | Type::Uint),
                    "enum input {name} has unsupported storage"
                );
                let mut tokens = BTreeSet::new();
                ensure!(
                    cases
                        .iter()
                        .all(|case| tokens.insert(case.to_ascii_lowercase())),
                    "enum input {name}: case tokens collide after lowercasing"
                );
                field.enumeration = Some(InputEnum {
                    ty,
                    cases: cases.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn enum_tokens_preserve_explicit_values_and_author_defaults() -> Result<()> {
        let abi = Abi { aov_count: 0,
            context: Abi::spec_context(),
            closure: openpbr_fields(),
        };
        let source = "// enum values are intentionally nonsequential\nenum Finish { Matte = 17, Glossy = -3 }; struct Inputs { Finish finish=Finish::Glossy; }; Closure surface(ShadingContext ctx, Inputs inputs) { Closure c={}; c.base_weight=float(int(inputs.finish)); return c; }";
        let program = Compiler::discover().compile_surface(
            &abi,
            source,
            Path::new("enum-test.slang"),
            "enumtest",
        )?;
        let layout = &program.inputs;
        assert_eq!(
            layout.fields[0]
                .enumeration
                .as_ref()
                .context("enum metadata")?
                .cases,
            ["Matte", "Glossy"]
        );
        for (token, index) in [("matte", 0), ("glossy", 1)] {
            let words = layout.pack(
                &[("finish".into(), Value::from(token))].into(),
                |_, _| unreachable!(),
            )?;
            assert_eq!(words[layout.fields[0].offset], index);
        }
        for bad in [Value::from(17), Value::from("Glossy"), Value::from("other")] {
            assert!(
                layout
                    .pack(&[("finish".into(), bad)].into(), |_, _| unreachable!())
                    .is_err()
            );
        }
        assert_eq!(layout.pack(&BTreeMap::new(), |_, _| unreachable!())?[0], 0);
        Ok(())
    }
}
