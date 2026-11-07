//! Conservative compiler-input identity. Slang owns lexing, preprocessing and
//! import resolution. Repros retain the bytes actually read, including failed
//! resolution probes; native code generation replays that same filesystem.
use super::*;

pub struct PreparedModules {
    work: Workspace,
    executable: PathBuf,
    module: PathBuf,
    main: PathBuf,
    replay_source: bool,
    args: Vec<String>,
    entries: Vec<(String, String)>,
    snapshot: Snapshot,
    identity: Vec<u8>,
}

/// A cheap exact-input validation receipt, stored by the renderer in its
/// ordinary content-addressed store. Failed resolver probes are dependencies
/// too: a new higher-priority file must force compiler resolution again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputIdentity {
    pub identity: Vec<u8>,
    reads: BTreeMap<PathBuf, Option<Vec<u8>>>,
    reusable: bool,
    resolved: BTreeMap<PathBuf, PathBuf>,
    canonical: BTreeMap<PathBuf, PathBuf>,
}
impl InputIdentity {
    pub fn valid(&self) -> bool {
        self.reusable && self.current()
    }
    fn current(&self) -> bool {
        self
                .reads
                .iter()
                .all(|(path, bytes)| fs::read(path).ok().as_ref() == bytes.as_ref())
            && self
                .canonical
                .iter()
                .all(|(path, target)| path.canonicalize().ok().as_ref() == Some(target))
    }
    /// A portable proof key must cover every resolver input of code generation,
    /// including probes introduced by a generated wrapper. Native composition
    /// separately permits target-only inputs through `verify`.
    pub fn verify_proof(&self, expected: &Self) -> Result<()> {
        self.verify(expected)?;
        ensure!(self.reads.iter().all(|(path,bytes)|expected.reads.get(path)==Some(bytes)),
            "Slang proof read inputs absent from its compiler identity; cannot cache the result");
        Ok(())
    }

    /// Compare shared imported inputs, including unsuccessful search probes
    /// and aliases that resolve to an already loaded module.
    pub fn verify(&self, expected: &Self) -> Result<()> {
        for (path, bytes) in &self.reads {
            if let Some(previous) = expected.reads.get(path) {
                ensure!(
                    bytes == previous,
                    "Slang import {} changed during compilation; retry the command",
                    path.display()
                );
            }
        }
        for (path, target) in &self.resolved {
            if let Some(previous) = expected.resolved.get(path) {
                ensure!(
                    target == previous,
                    "Slang import resolution {} changed during compilation; retry the command",
                    path.display()
                );
            }
        }
        for (path, target) in &self.canonical {
            if let Some(previous) = expected.canonical.get(path) {
                ensure!(
                    target == previous,
                    "Slang import resolution {} changed during compilation; retry the command",
                    path.display()
                );
            }
        }
        Ok(())
    }
}

struct Snapshot {
    repro: PathBuf,
    files: BTreeMap<PathBuf, Vec<u8>>,
    reads: BTreeMap<PathBuf, Option<Vec<u8>>>,
    uncertain: Vec<PathBuf>,
    resolved: BTreeMap<PathBuf, PathBuf>,
    canonical: BTreeMap<PathBuf, PathBuf>,
}

/// Capture each actual proof pass, not a post-compilation read of its depfile.
/// Shared imports must agree between passes, including failed search probes.
pub(super) fn check_imports(compiler: &Compiler, work: &Workspace, repro: PathBuf) -> Result<()> {
    let receipt = Snapshot::read(compiler, repro)?.receipt(work, Vec::new());
    if let Some(expected) = &compiler.expected { receipt.verify(expected)?; }
    if let Some(captured) = &compiler.captured {
        let mut captured = captured.lock().unwrap();
        if let Some(previous) = captured.as_mut() {
            receipt.verify(previous)?;
            previous.reads.extend(receipt.reads);
            previous.resolved.extend(receipt.resolved);
            previous.canonical.extend(receipt.canonical);
            previous.reusable &= receipt.reusable;
        } else {
            *captured = Some(receipt);
        }
    }
    Ok(())
}

fn part(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

impl Snapshot {
    fn read(compiler: &Compiler, repro: PathBuf) -> Result<Self> {
        let directory = repro.with_extension("");
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        let output = compiler_output(
            Command::new(&compiler.executable)
                .arg("-extract-repro")
                .arg(&repro),
        )?;
        ensure!(
            output.status.success(),
            "extract Slang dependency snapshot: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let manifest = read_compiler_text(directory.join("manifest.txt"))?;
        Self::read_manifest(repro, &directory, &manifest)
    }

    fn read_manifest(repro: PathBuf, directory: &Path, manifest: &str) -> Result<Self> {
        let mut files = BTreeMap::new();
        let mut extracted = BTreeMap::new();
        let mut reads = BTreeMap::new();
        let mut uncertain = Vec::new();
        let mut uncertain_details = Vec::new();
        let mut resolved = BTreeMap::new();
        let mut section = "";
        for line in manifest.lines() {
            if line.starts_with('[') {
                section = line;
                continue;
            }
            if section == "[files]" {
                let (name, path) = line
                    .split_once(" -> ")
                    .context("Slang snapshot file mapping")?;
                let bytes = fs::read(directory.join(name))?;
                let path = PathBuf::from(path);
                extracted.insert(name.to_owned(), (path.clone(), bytes.clone()));
                reads.insert(path.clone(), Some(bytes.clone()));
                files.insert(path, bytes);
            } else if section == "[paths]" {
                let (path, name) = line
                    .split_once(" -> ")
                    .context("Slang snapshot resolution mapping")?;
                let bytes = if matches!(name, "[not loaded]" | "[not found]") {
                    None
                } else if let Some((target, bytes)) = extracted.get(name) {
                    resolved.insert(PathBuf::from(path), target.clone());
                    Some(bytes.clone())
                } else if let Some(bytes) = reads.get(&PathBuf::from(path)) {
                    bytes.clone()
                } else {
                    uncertain.push(PathBuf::from(path));
                    uncertain_details.push(line.to_owned());
                    continue;
                };
                reads.insert(PathBuf::from(path), bytes);
            }
        }
        let canonical = reads
            .iter()
            .filter_map(|(path, bytes)| {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    repro.parent().unwrap().join(path)
                };
                bytes.as_ref().and_then(|_| {
                    absolute
                        .canonicalize()
                        .ok()
                        .map(|target| (absolute, target))
                })
            })
            .collect();
        // An unrecognized external resolver record cannot be omitted from an
        // identity. Fail closed instead of guessing its binding or caching a
        // partial dependency graph.
        let root = repro.parent().context("Slang snapshot directory")?;
        ensure!(
            uncertain.iter().all(|path| {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                };
                absolute.starts_with(root)
                    && !path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
            }),
            "unrecognized external Slang dependency snapshot mapping; cannot establish a safe cache identity: {uncertain_details:?}"
        );
        Ok(Self {
            repro,
            files,
            reads,
            uncertain,
            resolved,
            canonical,
        })
    }
}

impl Snapshot {
    fn receipt(&self, work: &Workspace, identity: Vec<u8>) -> InputIdentity {
        let locate = |path: &Path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                work.0.join(path)
            }
        };
        let reusable = !self
            .uncertain
            .iter()
            .any(|p| !locate(p).starts_with(&work.0))
            && !self.reads.keys().any(|p| {
                locate(p).starts_with(&work.0)
                    && p.components().any(|c| c == std::path::Component::ParentDir)
            });
        let reads: BTreeMap<PathBuf, Option<Vec<u8>>> = self
            .reads
            .iter()
            .filter_map(|(path, bytes)| {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    work.0.join(path)
                };
                (!path.starts_with(&work.0)).then(|| (path, bytes.clone()))
            })
            .chain(
                self.files
                    .iter()
                    .filter(|(p, _)| !p.starts_with(&work.0))
                    .map(|(p, b)| (p.clone(), Some(b.clone()))),
            )
            .collect();
        let resolved: BTreeMap<_, _> = self
            .resolved
            .iter()
            .filter(|(p, _)| !locate(p).starts_with(&work.0))
            .map(|(p, t)| (locate(p), locate(t)))
            .collect();
        let canonical: BTreeMap<_, _> = self
            .canonical
            .iter()
            .filter(|(path, _)| !path.starts_with(&work.0))
            .map(|(p, t)| (p.clone(), t.clone()))
            .collect();
        let reusable = reusable
            && resolved.iter().all(|(p, t)| {
                canonical
                    .get(p)
                    .is_some_and(|c| t.canonicalize().ok().as_ref() == Some(c))
            });
        InputIdentity {
            identity,
            reads,
            reusable,
            resolved,
            canonical,
        }
    }
}

impl Compiler {
    pub fn input_policy(&self) -> String {
        format!("snapshot-v3-checked-module-imports:{VERSION}:{}", self.executable.display())
    }
    pub fn with_identity(&self, identity: &InputIdentity) -> Self {
        Self {
            executable: self.executable.clone(),
            expected: Some(identity.clone()),
            captured: self.captured.clone(),
        }
    }
    /// The current exact receipt associated with a caller's normalized proof key.
    pub fn input_identity(&self) -> Option<&InputIdentity> {
        self.expected.as_ref()
    }

    /// Run a proof against checked compiler input receipts. Changes between
    /// passes or before publication fail rather than caching mismatched output.
    pub fn capture_inputs<T>(&self, build: impl FnOnce(&Compiler) -> Result<T>) -> Result<(T, InputIdentity)> {
        let captured = std::sync::Arc::new(Mutex::new(None));
        let compiler = Self {
            executable: self.executable.clone(), expected: self.expected.clone(),
            captured: Some(captured.clone()),
        };
        let result = build(&compiler)?;
        let receipt = captured.lock().unwrap().take().context("proof did not capture Slang inputs")?;
        ensure!(receipt.current(), "Slang inputs changed during compilation; retry the command");
        Ok((result, receipt))
    }

    /// Resolve and type-check a native entry without downstream optimization.
    /// Supplied modules retain conservative text identity; only reachable ones
    /// participate. External imports use compiler tokens where safe.
    pub fn prepare_modules(
        &self,
        modules: &[(&str, &str)],
        main: &str,
        entries: &[(&str, &str)],
        args: &[&str],
    ) -> Result<PreparedModules> {
        self.prepare_identity(modules, main, entries, args, None, &BTreeMap::new())
    }

    /// Generated material modules have compiler-proven input identities. Use
    /// those rather than allowing original comments in their retained source
    /// to re-enter native keys. Their generator/compiler policy is covered by
    /// the renderer's shader namespace; native-only dependencies are still
    /// resolved and keyed by this request.
    pub fn prepare_identified_modules(
        &self,
        modules: &[(&str, &str)],
        main: &str,
        entries: &[(&str, &str)],
        args: &[&str],
        identities: &BTreeMap<String, Vec<u8>>,
    ) -> Result<PreparedModules> {
        self.prepare_identity(modules, main, entries, args, None, identities)
    }

    fn prepare_identity(
        &self,
        modules: &[(&str, &str)],
        main: &str,
        entries: &[(&str, &str)],
        args: &[&str],
        normalize_root: Option<bool>,
        identities: &BTreeMap<String, Vec<u8>>,
    ) -> Result<PreparedModules> {
        let _progress = crate::progress::Job::start(false);
        self.version()?;
        let work = Workspace::new()?;
        for (name, source) in modules {
            let path = work.0.join(format!("{name}.slang"));
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, source)?;
        }
        let main = work.0.join(format!("{main}.slang"));
        let repro = work.0.join("capture.repro");
        let depfile = work.0.join("capture.d");
        let mut command = Command::new(&self.executable);
        command
            .arg(&main)
            .current_dir(&work.0)
            .arg(INITIALIZATION)
            .arg("-I")
            .arg(&work.0)
            .args(args);
        for (entry, stage) in entries {
            command.args(["-entry", entry, "-stage", stage]);
        }
        let output = compiler_output(
            command
                .args(["-no-codegen", "-notes-disable", "88", "-o"])
                .arg(work.0.join("capture.slang-module"))
                .arg("-depfile")
                .arg(&depfile)
                .arg("-dump-repro")
                .arg(&repro),
        )?;
        ensure!(
            output.status.success(),
            "Slang identity: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let snapshot = Snapshot::read(self, repro)?;
        let mut dependencies: Vec<_> = snapshot.files.keys().cloned().collect();
        dependencies.sort_by_key(|path| path.strip_prefix(&work.0).unwrap_or(path).to_path_buf());
        let mut identity = Vec::new();
        part(&mut identity, if normalize_root.is_some() { b"slang-input-snapshot-v1" } else { b"slang-input-snapshot-v3-checked-module-imports" });
        part(&mut identity, VERSION.as_bytes());
        part(&mut identity, INITIALIZATION.as_bytes());
        let external = snapshot.files.keys().any(|path| !path.starts_with(&work.0));
        let mut options = args.iter();
        while let Some(arg) = options.next() {
            // A wholly embedded closure does not depend on unused external
            // search directories. This keeps bundled programs portable across
            // working directories. Exact-input receipts still validate lookups.
            if !external && *arg == "-I" {
                options.next();
                continue;
            }
            part(&mut identity, arg.as_bytes());
        }
        for (entry, stage) in entries {
            part(&mut identity, entry.as_bytes());
            part(&mut identity, stage.as_bytes());
        }
        for path in dependencies {
            let bytes = snapshot
                .files
                .get(&path)
                .with_context(|| format!("Slang snapshot missing {}", path.display()))?;
            let supplied = path.strip_prefix(&work.0).ok();
            part(
                &mut identity,
                supplied.unwrap_or(&path).to_string_lossy().as_bytes(),
            );
            if let Some(relative) = supplied {
                let name = relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(proven) = identities.get(&name) {
                    part(&mut identity, b"compiler-proven-material-module");
                    part(&mut identity, proven);
                    continue;
                }
            }
            let root = path == main;
            // -E is a token stream, not a serialization of preprocessor state.
            // Directives may affect that state without emitting tokens. Keep
            // their raw identity, and do the same for any diagnostic. This is
            // deliberately conservative (even a '#' in a string falls back).
            // Proof diagnostics are part of the cached Program. Native cache
            // payloads contain code only: an unrelated renderer warning must
            // not make a comment in an imported helper invalidate that code.
            let eligible = (normalize_root.is_none() || output.stderr.is_empty())
                && if root && normalize_root.is_some() {
                    normalize_root.unwrap()
                } else {
                    supplied.is_none() && !bytes.contains(&b'#')
                };
            if supplied.is_none() {
                reject_reserved(std::str::from_utf8(bytes)?, &path.display().to_string())?;
            }
            if eligible {
                let mut command = Command::new(&self.executable);
                command
                    .arg("-repro-file-system")
                    .arg(&snapshot.repro)
                    .arg(&path)
                    .current_dir(&work.0)
                    .arg(INITIALIZATION)
                    .arg("-I")
                    .arg(&work.0)
                    .args(args)
                    .args(["-E", "-line-directive-mode", "none"]);
                let tokens = compiler_output(&mut command)?;
                ensure!(
                    tokens.status.success(),
                    "preprocess Slang identity: {}",
                    String::from_utf8_lossy(&tokens.stderr)
                );
                if tokens.stderr.is_empty() {
                    part(&mut identity, &tokens.stdout);
                } else {
                    part(&mut identity, bytes);
                }
            } else {
                part(&mut identity, bytes);
            }
        }
        let mut bindings: Vec<_> = snapshot
            .resolved
            .iter()
            .map(|(path, target)| {
                (
                    path.strip_prefix(&work.0).unwrap_or(path).to_path_buf(),
                    target.strip_prefix(&work.0).unwrap_or(target).to_path_buf(),
                )
            })
            .collect();
        bindings.sort();
        for (path, target) in bindings {
            part(&mut identity, path.to_string_lossy().as_bytes());
            part(&mut identity, target.to_string_lossy().as_bytes());
        }
        // Serialized modules can misbind imports with the same leaf filename
        // (for example std/procedurals and blender/procedurals) when loading a
        // larger shader graph. Replay source against the same captured inputs
        // for these graphs; unique-name graphs retain checked-module reuse.
        let mut names = std::collections::BTreeSet::new();
        let replay_source = snapshot.files.keys()
            .filter(|path| path.extension().is_some_and(|ext| ext == "slang"))
            .any(|path| !names.insert(path.file_name().unwrap().to_owned()));
        let module = work.0.join("capture.slang-module");
        Ok(PreparedModules {
            work,
            executable: self.executable.clone(),
            module,
            main,
            replay_source,
            args: args.iter().map(|s| s.to_string()).collect(),
            entries: entries
                .iter()
                .map(|(e, s)| (e.to_string(), s.to_string()))
                .collect(),
            snapshot,
            identity: blake3::hash(&identity).as_bytes().to_vec(),
        })
    }

    /// Surface proofs share the same normalized root and resolved imports.
    /// The returned compiler checks every proof subprocess's captured imports
    /// against this snapshot before returning a publishable result.
    pub fn prepare_program(
        &self,
        abi: &Abi,
        source: &str,
        origin: &Path,
    ) -> Result<PreparedProgram> {
        self.prepare_bound_program(abi,source,origin,"")
    }

    /// Generated compute bindings are trusted compiler input, separate from
    /// authored source so its reserved-name checks stay identical to compilation.
    pub fn prepare_bound_program(
        &self, abi: &Abi, source: &str, origin: &Path, bindings: &str,
    ) -> Result<PreparedProgram> {
        let work = Workspace::new()?;
        write_modules(&work, abi, source)?;
        let mut modules = Vec::new();
        fn visit(root: &Path, path: &Path, modules: &mut Vec<(String, String)>) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let path = entry?.path();
                if path.is_dir() {
                    visit(root, &path, modules)?;
                } else {
                    modules.push((
                        path.strip_prefix(root)?
                            .with_extension("")
                            .to_string_lossy()
                            .replace('\\', "/"),
                        fs::read_to_string(path)?,
                    ));
                }
            }
            Ok(())
        }
        visit(&work.0, &work.0, &mut modules)?;
        let program=program_text(source,origin);
        let program=if bindings.is_empty() {program} else {format!("import vork;\n{bindings}\n{program}")};
        modules.push(("program".into(), program));
        let includes = include_paths(origin);
        let mut args = vec!["-target", "wgsl"];
        let include_strings: Vec<_> = includes
            .iter()
            .map(|p| std::env::current_dir().map(|cwd| cwd.join(p).display().to_string()))
            .collect::<std::io::Result<_>>()?;
        for include in &include_strings {
            args.extend(["-I", include]);
        }
        let refs: Vec<_> = modules
            .iter()
            .map(|(n, s)| (n.as_str(), s.as_str()))
            .collect();
        let prepared = self.prepare_identity(
            &refs,
            "program",
            &[],
            &args,
            Some(!source.contains('#') && !bindings.contains('#')),
            &BTreeMap::new(),
        )?;
        let expected = prepared.receipt();
        Ok(PreparedProgram {
            compiler: Compiler {
                executable: self.executable.clone(),
                expected: Some(expected),
                captured: self.captured.clone(),
            },
            prepared,
        })
    }
}

pub struct PreparedProgram {
    compiler: Compiler,
    prepared: PreparedModules,
}
impl PreparedProgram {
    pub fn identity(&self) -> &[u8] {
        self.prepared.identity()
    }
    pub fn compiler(&self) -> &Compiler {
        &self.compiler
    }
    pub fn receipt(&self) -> InputIdentity {
        self.prepared.receipt()
    }
}

impl PreparedModules {
    pub fn identity(&self) -> &[u8] {
        &self.identity
    }
    pub fn receipt(&self) -> InputIdentity {
        self.snapshot.receipt(&self.work, self.identity.clone())
    }
    pub fn compile(&self) -> Result<Vec<u8>> {
        let _progress = crate::progress::Job::start(false);
        let output = self.work.0.join("out");
        let mut command = Command::new(&self.executable);
        // Reuse the checked root where imported leaf names are unambiguous;
        // otherwise replay source. Both paths use the captured filesystem.
        // This module is request-local, never persisted.
        // Options must precede the serialized input: Slang loads its imports
        // while parsing that argument, before seeing any later defines.
        command
            .arg("-repro-file-system")
            .arg(&self.snapshot.repro)
            .current_dir(&self.work.0)
            .arg(INITIALIZATION)
            .arg("-I")
            .arg(&self.work.0)
            .args(&self.args)
            .arg(if self.replay_source { &self.main } else { &self.module });
        for (entry, stage) in &self.entries {
            command.args(["-entry", entry, "-stage", stage]);
        }
        let metal = self.args.windows(2).any(|a| a == ["-target", "metal"]);
        if metal && !self.args.iter().any(|a| a == "-line-directive-mode") {
            command.args(["-line-directive-mode", "none"]);
        }
        let result = compiler_output(command.arg("-o").arg(&output))?;
        ensure!(
            result.status.success(),
            "slangc {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
        let mut code = fs::read(output)?;
        if metal
            && self.args.windows(2).any(|a| {
                matches!(a[0].as_str(), "-fp-mode" | "-floating-point-mode") && a[1] == "precise"
            })
        {
            let mut prefix =
                b"#pragma METAL fp math_mode(safe)\n#pragma METAL fp contract(off)\n".to_vec();
            prefix.append(&mut code);
            code = prefix;
        }
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_missing_import_probes_invalidate_when_a_file_appears() -> Result<()> {
        let work = Workspace::new()?;
        let external = Workspace::new()?;
        let missing = external.0.join("higher_priority.slang");
        let manifest = format!("[files]\n[paths]\n{} -> [not found]\n", missing.display());
        let snapshot = Snapshot::read_manifest(work.0.join("input.slang-repro"), &work.0, &manifest)?;
        let receipt = snapshot.receipt(&work, vec![]);
        assert!(receipt.valid());
        fs::write(&missing, "export static const float value = 2.;")?;
        assert!(!receipt.valid(), "a newly resolved import must invalidate the receipt");
        let manifest = format!("[files]\n[paths]\n{} -> [unknown]\n", missing.display());
        assert!(Snapshot::read_manifest(work.0.join("input.slang-repro"), &work.0, &manifest).is_err());
        Ok(())
    }

    #[test]
    fn proof_capture_rejects_import_changes_between_passes() -> Result<()> {
        let work=Workspace::new()?;
        let helper=work.0.join("helper.slang");
        fs::write(&helper,"public float value(){return 0.25;}")?;
        let origin=work.0.join("main.slang");
        let source="import helper; struct Inputs {}; float3 display(DisplayContext ctx, Inputs inputs){return float3(value());}";
        let compiler=Compiler::discover();
        let result=compiler.capture_inputs(|compiler| {
            compiler.compile_display(&Abi { aov_count: 0, context: Abi::spec_context(), closure: openpbr_fields() },source,&origin,"first")?;
            fs::write(&helper,"public float value(){return 0.75;}")?;
            compiler.compile_display(&Abi { aov_count: 0, context: Abi::spec_context(), closure: openpbr_fields() },source,&origin,"second")
        });
        assert!(format!("{:#}",result.unwrap_err()).contains("changed during compilation"));
        Ok(())
    }

    const SOURCE: &str = "struct Inputs {}; Closure surface(ShadingContext ctx, Inputs inputs) { Closure c; c.base_color=float3(value()); return c; }";

    #[test]
    fn same_leaf_library_imports_replay_captured_source() -> Result<()> {
        let compiler = Compiler::discover();
        let modules = [
            ("main", "import std.procedurals; import blender.procedurals; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){float3 p=float3(.1,.2,.3);result[id.x]=magic(p).color.x+brick(p).color.x+wave(p);}"),
            ("std/procedurals", module_source("std/procedurals").unwrap()),
            ("blender/procedurals", module_source("blender/procedurals").unwrap()),
        ];
        for target in ["spirv", "metal"] {
            let args = ["-target", target, "-O2", "-fp-mode", "precise"];
            let entries = [("main", "compute")];
            let prepared = compiler.prepare_modules(&modules, "main", &entries, &args)?;
            assert!(prepared.replay_source, "distinct procedural modules must retain their bindings");
            assert_eq!(prepared.compile()?, compiler.compile_modules(&modules, "main", &entries, &args)?, "{target}: captured replay must preserve imported functions");
        }
        Ok(())
    }

    #[test]
    fn checked_native_module_matches_source_codegen() -> Result<()> {
        let compiler = Compiler::discover();
        let modules = [
            ("main", "import helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){float x; result[id.x]=value()+x;}"),
            ("helper", "public float value(){return 0.25;}"),
        ];
        for target in ["spirv", "metal"] {
            let args = ["-target", target, "-O2", "-fp-mode", "precise"];
            let entries = [("main", "compute")];
            let prepared = compiler.prepare_modules(&modules, "main", &entries, &args)?;
            assert_eq!(prepared.compile()?, compiler.compile_modules(&modules, "main", &entries, &args)?, "{target}: checked module must preserve codegen, including initialization and precise math");
        }
        Ok(())
    }

    #[test]
    fn program_tokens_reuse_comments_but_invalidate_real_and_imported_changes() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        let origin = work.0.join("material.slang");
        let helper = work.0.join("helper.slang");
        fs::write(&helper, "public float value(){return 0.25;}\n")?;
        let source = format!("import helper; {SOURCE}");
        let abi = Abi {
            aov_count: 0,
            context: Abi::spec_context(),
            closure: openpbr_fields(),
        };
        let base = compiler.prepare_program(&abi, &source, &origin)?;
        let comment = compiler.prepare_program(
            &abi,
            &format!("// changed description\n{source}\n/* more */"),
            &origin,
        )?;
        assert!(
            base.identity() == comment.identity(),
            "root comment must hit"
        );
        fs::write(
            &helper,
            "// imported description\npublic float value() { return 0.25; }\n",
        )?;
        let comment = compiler.prepare_program(&abi, &source, &origin)?;
        assert!(
            base.identity() == comment.identity(),
            "imported comment must hit"
        );
        fs::write(&helper, "public float value(){return 0.75;}\n")?;
        let edited = compiler.prepare_program(&abi, &source, &origin)?;
        assert!(
            base.identity() != edited.identity(),
            "imported colour edit must miss"
        );
        let edited = compiler.prepare_program(
            &abi,
            &source.replace("float3(value())", "float3(0.5)"),
            &origin,
        )?;
        assert!(
            comment.identity() != edited.identity(),
            "root colour edit must miss"
        );
        fs::remove_file(&helper)?;
        assert!(
            compiler.prepare_program(&abi, &source, &origin).is_err(),
            "deleted import cannot hit"
        );
        fs::write(&helper, "public float value(){return 0.25;}\n")?;
        assert_eq!(
            base.identity(),
            compiler.prepare_program(&abi, &source, &origin)?.identity()
        );
        assert_ne!(
            base.identity(),
            compiler
                .prepare_program(&abi, &format!("#warning check me\n{source}"), &origin)?
                .identity(),
            "diagnostic cannot disappear behind a hit"
        );
        Ok(())
    }

    #[test]
    fn native_snapshot_tracks_resolution_and_codegen_uses_captured_bytes() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        let first = work.0.join("first");
        let second = work.0.join("second");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        let helper = second.join("helper.slang");
        fs::write(&helper, "public float value(){return 0.25;}")?;
        let main = "import helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let modules = [
            ("main", main),
            ("unused", "public float unrelated(){return 1.;}"),
        ];
        let first_text = first.display().to_string();
        let second_text = second.display().to_string();
        let args = ["-target", "spirv", "-I", &first_text, "-I", &second_text];
        let base = compiler.prepare_modules(&modules, "main", &[("main", "compute")], &args)?;
        let binary = base.compile()?;
        let mut artifacts = BTreeMap::new();
        artifacts.insert(base.identity().to_vec(), binary.clone());
        assert!(base.receipt().valid());
        fs::write(&helper, "// comment\npublic float value(){return 0.25;}")?;
        assert!(
            !base.receipt().valid(),
            "changed imported bytes must revalidate with the compiler"
        );
        let comment = compiler.prepare_modules(
            &[
                ("main", main),
                ("unused", "public float unrelated(){return 9.;}"),
            ],
            "main",
            &[("main", "compute")],
            &args,
        )?;
        assert!(
            base.identity() == comment.identity(),
            "unused modules and imported comments must hit"
        );
        assert_eq!(
            artifacts.get(comment.identity()),
            Some(&binary),
            "comment edit reuses the stored artifact"
        );
        fs::write(&helper, "public float value(){return 0.75;}")?;
        let edited = compiler.prepare_modules(&modules, "main", &[("main", "compute")], &args)?;
        assert_ne!(base.identity(), edited.identity());
        assert!(
            !artifacts.contains_key(edited.identity()),
            "real imported edit must miss"
        );
        assert_ne!(
            binary,
            edited.compile()?,
            "semantic import edit must produce a new artifact"
        );
        assert_eq!(
            binary,
            base.compile()?,
            "concurrent edit cannot change the keyed snapshot's code"
        );
        fs::write(
            first.join("helper.slang"),
            "public float value(){return 0.5;}",
        )?;
        assert!(
            !edited.receipt().valid(),
            "a previously failed resolution probe is now a file"
        );
        let shadowed = compiler.prepare_modules(&modules, "main", &[("main", "compute")], &args)?;
        assert_ne!(
            edited.identity(),
            shadowed.identity(),
            "new higher-priority file must invalidate"
        );
        assert_ne!(edited.compile()?, shadowed.compile()?);
        Ok(())
    }

    #[test]
    fn proven_generated_modules_do_not_reintroduce_original_comments() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        let abi = Abi {
            aov_count: 0,
            context: Abi::spec_context(),
            closure: openpbr_fields(),
        };
        let origin = work.0.join("material.slang");
        let material = "public float value(){return 0.25;}";
        let main = "import material; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let proof = compiler.prepare_program(&abi, material, &origin)?;
        let ids = BTreeMap::from([("material".into(), proof.identity().to_vec())]);
        let args = ["-target", "spirv"];
        let base = compiler.prepare_identified_modules(
            &[("main", main), ("material", material)],
            "main",
            &[("main", "compute")],
            &args,
            &ids,
        )?;
        let binary = base.compile()?;
        let comment = format!("// retained source after a proof-cache rebuild\n{material}");
        let comment_proof = compiler.prepare_program(&abi, &comment, &origin)?;
        assert_eq!(proof.identity(), comment_proof.identity());
        let comment = compiler.prepare_identified_modules(
            &[("main", main), ("material", &comment)],
            "main",
            &[("main", "compute")],
            &args,
            &ids,
        )?;
        assert_eq!(base.identity(), comment.identity());
        let edit = material.replace("0.25", "0.75");
        let edited_proof = compiler.prepare_program(&abi, &edit, &origin)?;
        let ids = BTreeMap::from([("material".into(), edited_proof.identity().to_vec())]);
        let edited = compiler.prepare_identified_modules(
            &[("main", main), ("material", &edit)],
            "main",
            &[("main", "compute")],
            &args,
            &ids,
        )?;
        assert_ne!(base.identity(), edited.identity());
        assert_ne!(binary, edited.compile()?);
        Ok(())
    }

    #[test]
    fn unrelated_native_diagnostics_do_not_invalidate_imported_comments() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        let helper = work.0.join("helper.slang");
        fs::write(&helper, "public float value(){return 0.25;}")?;
        let include = work.0.display().to_string();
        let args = ["-target", "spirv", "-I", &include];
        let main = "#warning renderer diagnostic\nimport helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let base =
            compiler.prepare_modules(&[("main", main)], "main", &[("main", "compute")], &args)?;
        fs::write(
            &helper,
            "// imported description\npublic float value(){return 0.25;}",
        )?;
        let comment =
            compiler.prepare_modules(&[("main", main)], "main", &[("main", "compute")], &args)?;
        assert_eq!(base.identity(), comment.identity());
        Ok(())
    }

    #[test]
    fn target_only_imports_invalidate_native_artifacts_on_both_targets() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        fs::write(
            work.0.join("helper.slang"),
            "#if defined(NATIVE_LINK)\nimport leaf;\npublic float value(){return leaf_value();}\n#else\npublic float value(){return 0.25;}\n#endif\n",
        )?;
        let leaf = work.0.join("leaf.slang");
        let origin = work.0.join("material.slang");
        let source = format!("import helper; {SOURCE}");
        let abi = Abi {
            aov_count: 0,
            context: Abi::spec_context(),
            closure: openpbr_fields(),
        };
        let proof = compiler.prepare_program(&abi, &source, &origin)?;
        let main = "import helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let include = work.0.display().to_string();
        for target in ["spirv", "metal"] {
            fs::write(&leaf, "public float leaf_value(){return 0.25;}")?;
            let args = ["-target", target, "-DNATIVE_LINK=1", "-I", &include];
            let before = compiler.prepare_modules(
                &[("main", main)],
                "main",
                &[("main", "compute")],
                &args,
            )?;
            let binary = before.compile()?;
            fs::write(&leaf, "public float leaf_value(){return 0.75;}")?;
            assert!(!before.receipt().valid());
            let after = compiler.prepare_modules(
                &[("main", main)],
                "main",
                &[("main", "compute")],
                &args,
            )?;
            assert_ne!(before.identity(), after.identity());
            assert_ne!(
                binary,
                after.compile()?,
                "{target}: native-only import must change code"
            );
            assert_eq!(
                proof.identity(),
                compiler.prepare_program(&abi, &source, &origin)?.identity(),
                "the WGSL proof alone does not see this dependency"
            );
        }
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn changing_a_symlink_revalidates_even_when_its_bytes_match() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        let first = work.0.join("one");
        let second = work.0.join("two");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        for directory in [&first, &second] {
            fs::write(
                directory.join("helper.slang"),
                "public float value(){return 0.25;}",
            )?;
        }
        let link = work.0.join("selected");
        std::os::unix::fs::symlink(&first, &link)?;
        let include = link.display().to_string();
        let args = ["-target", "spirv", "-I", &include];
        let main = "import helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let base =
            compiler.prepare_modules(&[("main", main)], "main", &[("main", "compute")], &args)?;
        assert!(base.receipt().valid());
        let binary = base.compile()?;
        fs::remove_file(&link)?;
        std::os::unix::fs::symlink(&second, &link)?;
        assert!(
            !base.receipt().valid(),
            "matching file bytes do not prove the same resolution"
        );
        fs::write(
            second.join("helper.slang"),
            "public float value(){return 0.75;}",
        )?;
        let edited =
            compiler.prepare_modules(&[("main", main)], "main", &[("main", "compute")], &args)?;
        assert_ne!(base.identity(), edited.identity());
        assert_ne!(binary, edited.compile()?);
        Ok(())
    }

    #[test]
    fn proof_cannot_publish_against_an_old_dependency_snapshot() -> Result<()> {
        let compiler = Compiler::discover();
        let work = Workspace::new()?;
        fs::write(
            work.0.join("helper.slang"),
            "public float value(){return 0.25;}",
        )?;
        let source = format!("import helper; {SOURCE}");
        let origin = work.0.join("material.slang");
        let abi = Abi {
            aov_count: 0,
            context: Abi::spec_context(),
            closure: openpbr_fields(),
        };
        let prepared = compiler.prepare_program(&abi, &source, &origin)?;
        fs::write(
            work.0.join("helper.slang"),
            "public float value(){return 0.75;}",
        )?;
        let error = prepared
            .compiler()
            .stages(&abi, &source, &origin)
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("changed during compilation"),
            "{error:#}"
        );
        Ok(())
    }
}
