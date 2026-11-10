//! Keep native Blend replay scratch local to its owning surface wrapper.
//!
//! The source ABI uses invocation-private globals so authored expression helpers
//! can participate in replay. Each complete native wrapper resets that scratch;
//! no value is communicated between wrappers. Sharing the variables in SPIR-V
//! nevertheless hides the initial metadata pass's dead closure work from Slang's
//! optimizer. Separate their storage before running the pinned optimizer's local
//! propagation passes. Function control flags and floating-point rules survive.
use super::{Compiler, compiler_output};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void},
    path::PathBuf,
    process::Command,
};

const STATE: &[&str] = &[
    "vork_node_cursor",
    "vork_target_node",
    "vork_target_index",
    "vork_target_energy",
    "vork_leaf_scale",
    "vork_leaf_opacity",
    "vork_root_opacity",
    "vork_root_energy",
    "vork_random_leaf",
    "vork_found_leaf",
    "vork_selected_closure",
];

#[derive(Clone, Copy)]
struct Instruction {
    at: usize,
    len: usize,
    op: u32,
}

// Address operands in the pinned compiler's logical SPIR-V. In particular,
// composite member numbers and vector shuffle selectors are literals, not IDs.
// The other pointer-producing/consuming operations below are escape checks;
// only direct loads, stores and access chains are eligible for localization.
fn addresses(op: u32, len: usize) -> Vec<usize> {
    let operands = match op {
        61 | 65..=68 | 70 | 83 | 117 | 124 | 227 | 229..=242 | 318 | 400 => vec![3],
        62..=64 => vec![1, 2],
        57 => (4..len).collect(),
        12 => (5..len).collect(),
        59 => vec![4],
        169 => vec![4, 5],
        245 => (3..len).step_by(2).collect(),
        254 | 256 | 257 | 228 | 319 => vec![1],
        401..=403 => vec![3, 4],
        _ => Vec::new(),
    };
    operands.into_iter().filter(|&i| i < len).collect()
}

/// Return None for a module whose scratch escapes the known wrapper contract.
/// In that case the ordinary, unmodified shader remains the correct fallback.
fn separate(code: &[u8]) -> Result<Option<Vec<u32>>> {
    ensure!(
        code.len() >= 20 && code.len() % 4 == 0,
        "invalid native SPIR-V size"
    );
    let words: Vec<u32> = code
        .chunks_exact(4)
        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    ensure!(words[0] == 0x07230203, "invalid native SPIR-V magic");
    if words[1] != 0x00010500 {
        return Ok(None); // The pinned native target and its entry-interface rules.
    }
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let len = (words[at] >> 16) as usize;
        ensure!(
            len > 0 && len <= words.len() - at,
            "invalid native SPIR-V instruction"
        );
        instructions.push(Instruction {
            at,
            len,
            op: words[at] & 65535,
        });
        at += len;
    }
    let mut names = BTreeMap::new();
    let mut entries = BTreeSet::new();
    let mut constants = BTreeSet::new();
    for i in &instructions {
        let w = &words[i.at..i.at + i.len];
        match i.op {
            // This pass understands the logical-pointer subset used by native
            // path shaders. New capabilities (especially variable/physical
            // pointers) require an operand/escape audit before localization.
            17 if i.len != 2 || !matches!(w[1], 1 | 49 | 50 | 4472 | 5301 | 5302 | 5307 | 5308) => {
                return Ok(None);
            }
            14 if i.len != 3 || w[1] != 0 => return Ok(None),
            5 if i.len >= 3 => {
                let bytes: Vec<u8> = w[2..]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .take_while(|v| *v != 0)
                    .collect();
                names.insert(
                    w[1],
                    String::from_utf8(bytes).context("native SPIR-V name")?,
                );
            }
            15 if i.len >= 4 => {
                entries.insert(w[2]);
            }
            41..=44 | 46 if i.len >= 3 => {
                constants.insert(w[2]);
            }
            _ => {}
        }
    }
    let state: BTreeSet<u32> = names
        .iter()
        .filter(|(_, n)| STATE.contains(&n.as_str()))
        .map(|(&id, _)| id)
        .collect();
    if state.is_empty() {
        return Ok(None);
    }
    let mut declarations = BTreeMap::new();
    let mut initial = BTreeMap::new();
    let mut uses: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut calls: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut function = None;
    for i in &instructions {
        let w = &words[i.at..i.at + i.len];
        match i.op {
            59 if i.len >= 4 && state.contains(&w[2]) => {
                if w[3] != 6 || i.len > 5 {
                    return Ok(None);
                } // Private storage only.
                declarations.insert(w[2], w.to_vec());
                if i.len == 5 {
                    initial.insert(w[2], w[4]);
                }
            }
            // Do not discard variable-specific precision or aliasing decorations.
            71 | 332 if i.len >= 2 && state.contains(&w[1]) => return Ok(None),
            74 | 75 if w[1..].iter().any(|id| state.contains(id)) => return Ok(None),
            54 if i.len >= 5 => {
                function = Some(w[2]);
            }
            56 => {
                function = None;
            }
            _ => {}
        }
        let Some(f) = function else {
            continue;
        };
        if i.op == 57 && i.len >= 4 {
            calls.entry(f).or_default().insert(w[3]);
        }
        for operand in addresses(i.op, i.len) {
            let id = w[operand];
            if !state.contains(&id) {
                continue;
            }
            // Only direct address uses are rewritten. Escaping scratch retains
            // the original module and its original shared-storage semantics.
            let address = matches!((i.op, operand), (61, 3) | (62, 1) | (65 | 66, 3));
            if !address {
                return Ok(None);
            }
            if entries.contains(&f) {
                if i.op != 62 || operand != 1 || i.len != 3 || !constants.contains(&w[2]) {
                    return Ok(None);
                }
                if initial.insert(id, w[2]).is_some_and(|old| old != w[2]) {
                    return Ok(None);
                }
            } else if names
                .get(&f)
                .and_then(|n| n.strip_prefix("vork_surface_"))
                .is_some_and(|tag| tag.len() == 12 && tag.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                uses.entry(f).or_default().insert(id);
            } else {
                return Ok(None);
            }
        }
    }
    if uses.is_empty()
        || uses
            .values()
            .flatten()
            .any(|id| !declarations.contains_key(id) || !initial.contains_key(id))
    {
        return Ok(None);
    }
    // Distinct owners must not communicate through the scratch. In particular,
    // a user helper with a wrapper-like name is not another independent owner.
    for &owner in uses.keys() {
        let mut pending = vec![owner];
        let mut visited = BTreeSet::new();
        while let Some(f) = pending.pop() {
            if !visited.insert(f) {
                continue;
            }
            for &callee in calls.get(&f).into_iter().flatten() {
                if uses.contains_key(&callee) {
                    return Ok(None);
                }
                pending.push(callee);
            }
        }
    }
    let mut bound = words[3];
    let mut maps = BTreeMap::new();
    let mut clones = Vec::new();
    for (&f, vars) in &uses {
        let mut map = BTreeMap::new();
        for &id in vars {
            let new = bound;
            bound = bound
                .checked_add(1)
                .context("native SPIR-V ID bound overflow")?;
            map.insert(id, new);
            let mut declaration = declarations[&id][..4].to_vec();
            declaration[0] = (5 << 16) | 59;
            declaration[2] = new;
            declaration.push(initial[&id]);
            clones.extend(declaration);
        }
        maps.insert(f, map);
    }
    let ids: Vec<_> = maps
        .values()
        .flat_map(|map| map.values().copied())
        .collect();
    let mut result = words[..5].to_vec();
    result[3] = bound;
    let mut inserted = false;
    function = None;
    for i in &instructions {
        let w = &words[i.at..i.at + i.len];
        if i.op == 54 {
            if !inserted {
                result.extend_from_slice(&clones);
                inserted = true;
            }
            function = Some(w[2]);
        }
        if i.op == 15 {
            ensure!(
                i.len + ids.len() <= u16::MAX as usize,
                "native SPIR-V entry interface too large"
            );
            result.push((((i.len + ids.len()) as u32) << 16) | 15);
            result.extend_from_slice(&w[1..]);
            result.extend_from_slice(&ids);
        } else {
            let start = result.len();
            result.extend_from_slice(w);
            if let Some(map) = function.and_then(|f| maps.get(&f)) {
                let operand = match i.op {
                    61 | 65 | 66 => Some(3),
                    62 => Some(1),
                    _ => None,
                };
                if let Some(operand) = operand.filter(|&o| o < i.len) {
                    if let Some(&new) = map.get(&w[operand]) {
                        result[start + operand] = new;
                    }
                }
            }
        }
        if i.op == 56 {
            function = None;
        }
    }
    Ok(Some(result))
}

type Output = unsafe extern "C" fn(*const c_void, usize, *mut c_void);
// Pinned Slang 2026.21 glslang_CompileRequest_1_3. The optimizer is already part
// of the required Slang toolchain; this does not load a system SPIRV-Tools build.
#[repr(C)]
struct Request {
    size: usize,
    source: *const c_char,
    begin: *const c_void,
    end: *const c_void,
    diagnostic: Output,
    diagnostic_data: *mut c_void,
    output: Output,
    output_data: *mut c_void,
    stage: i32,
    action: u32,
    optimization: u32,
    debug: u32,
    target: *const c_char,
    version: [i32; 3],
    entry: *const c_char,
    flags: *const *const c_char,
    flag_count: usize,
}
unsafe extern "C" fn collect(data: *const c_void, len: usize, context: *mut c_void) {
    if len != 0 {
        unsafe {
            (&mut *context.cast::<Vec<u8>>())
                .extend_from_slice(std::slice::from_raw_parts(data.cast(), len));
        }
    }
}

impl Compiler {
    /// Specialize renderer-owned Blend scratch after native SPIR-V generation,
    /// before changing the driver's function-control hints or caching the code.
    pub fn localize_native_blends(&self, code: &[u8]) -> Result<Vec<u8>> {
        let Some(input) = separate(code)? else {
            return Ok(code.to_vec());
        };
        self.version()?;
        let output = compiler_output(Command::new(&self.executable).arg("-get-spirv-opt-path"))?;
        ensure!(output.status.success(), "locate pinned Slang optimizer");
        let text = String::from_utf8([output.stdout, output.stderr].concat())?;
        let path = text
            .lines()
            .find_map(|line| line.strip_prefix("spirv-opt path: "))
            .map(PathBuf::from)
            .context("pinned Slang did not report its optimizer library")?;
        let flags = [
            c"--private-to-local",
            c"--scalar-replacement=0",
            c"--ssa-rewrite",
            c"--ccp",
            c"--eliminate-dead-branches",
            c"--eliminate-dead-code-aggressive",
        ]
        .map(|flag| flag.as_ptr());
        let mut result = Vec::<u8>::new();
        let mut diagnostics = Vec::<u8>::new();
        let mut request = Request {
            size: std::mem::size_of::<Request>(),
            source: c"native Blend scratch".as_ptr(),
            begin: input.as_ptr().cast(),
            end: unsafe { input.as_ptr().add(input.len()).cast() },
            diagnostic: collect,
            diagnostic_data: (&mut diagnostics as *mut Vec<u8>).cast(),
            output: collect,
            output_data: (&mut result as *mut Vec<u8>).cast(),
            stage: 0,
            action: 2,
            optimization: 0,
            debug: 0,
            target: c"vulkan1.2".as_ptr(),
            version: [1, 5, 0],
            entry: std::ptr::null(),
            flags: flags.as_ptr(),
            flag_count: flags.len(),
        };
        // The input, callbacks, output owners and loaded library all outlive this
        // synchronous call. The pinned C ABI neither retains nor frees them.
        unsafe {
            let library = libloading::Library::new(&path).context("load pinned Slang optimizer")?;
            let compile = library
                .get::<unsafe extern "C" fn(*mut Request) -> i32>(b"glslang_compile_1_3\0")?;
            let status = compile(&mut request);
            ensure!(
                status == 0 && !result.is_empty(),
                "localize native Blend scratch: {}",
                String::from_utf8_lossy(&diagnostics)
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(words: &mut Vec<u32>, op: u32, operands: &[u32]) {
        words.push(((operands.len() as u32 + 1) << 16) | op);
        words.extend_from_slice(operands);
    }
    fn name(words: &mut Vec<u32>, id: u32, text: &str) {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let mut operands = vec![id];
        operands.extend(
            bytes
                .chunks_exact(4)
                .map(|v| u32::from_le_bytes(v.try_into().unwrap())),
        );
        instruction(words, 5, &operands);
    }
    // A minimal instruction fixture exercises the transform; the ignored
    // compiler integration test below validates complete generated modules.
    fn fixture(extra_op: u32, extra: &[u32]) -> Vec<u8> {
        let mut w = vec![0x07230203, 0x00010500, 0, 100, 0];
        name(&mut w, 39, "vork_node_cursor");
        name(&mut w, 50, "vork_surface_000000000001");
        instruction(&mut w, 15, &[5, 40, 0]);
        instruction(&mut w, 43, &[1, 2, 0]);
        instruction(&mut w, 59, &[3, 39, 6]);
        instruction(&mut w, 54, &[1, 40, 0, 4]);
        instruction(&mut w, 62, &[39, 2]);
        instruction(&mut w, 56, &[]);
        instruction(&mut w, 54, &[1, 50, 2, 4]);
        instruction(&mut w, 61, &[1, 51, 39]);
        instruction(&mut w, extra_op, extra);
        instruction(&mut w, 56, &[]);
        w.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    #[test]
    fn localizes_addresses_without_rewriting_literal_member_indices() {
        let w = separate(&fixture(81, &[1, 52, 51, 39])).unwrap().unwrap();
        assert_eq!(w[3], 101);
        assert!(w.windows(4).any(|v| v == [(4 << 16) | 61, 1, 51, 100]));
        assert!(w.windows(5).any(|v| v == [(5 << 16) | 81, 1, 52, 51, 39]));
        assert!(w.windows(5).any(|v| v == [(5 << 16) | 54, 1, 50, 2, 4]));
    }
    #[test]
    fn escaped_scratch_keeps_original_shader() {
        assert!(separate(&fixture(57, &[1, 52, 60, 39])).unwrap().is_none());
        assert!(separate(&fixture(83, &[3, 52, 39])).unwrap().is_none());
    }
    #[test]
    fn coupled_wrappers_keep_shared_scratch() {
        let input = fixture(57, &[1, 52, 70]);
        let mut w: Vec<u32> = input
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        name(&mut w, 70, "vork_surface_000000000002");
        instruction(&mut w, 54, &[1, 70, 2, 4]);
        instruction(&mut w, 61, &[1, 71, 39]);
        instruction(&mut w, 56, &[]);
        assert!(
            separate(&w.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn new_pointer_capabilities_require_an_escape_audit() {
        let mut code = fixture(81, &[1, 52, 51, 39]);
        let capability = [(2u32 << 16) | 17, 4442]; // VariablePointers
        code.splice(20..20, capability.iter().flat_map(|v| v.to_le_bytes()));
        assert!(separate(&code).unwrap().is_none());
    }
    #[test]
    fn rejects_truncated_instructions() {
        let mut code = fixture(81, &[1, 52, 51, 39]);
        code.extend_from_slice(&((4u32 << 16) | 61).to_le_bytes());
        assert!(separate(&code).is_err());
    }
    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn generated_module() -> Result<()> {
        let compiler = Compiler::discover();
        let input = if let Ok(path) = std::env::var("VORK_BLEND_INPUT") {
            std::fs::read(path)?
        } else {
            compiler.compile_modules(
                &[(
                    "main",
                    r#"
static uint vork_node_cursor;
[noinline] uint vork_surface_000000000001(uint count) {
    vork_node_cursor=0;
    [loop] for(uint i=0;i<count;i++) {vork_node_cursor+=i;}
    return vork_node_cursor;
}
[noinline] uint vork_surface_000000000002(uint count) {
    vork_node_cursor=0;
    [loop] for(uint i=0;i<count;i++) {vork_node_cursor+=i*2;}
    return vork_node_cursor;
}
RWStructuredBuffer<uint> values;
[shader("compute")][numthreads(1,1,1)] void main() {
    values[0]=vork_surface_000000000001(values[1])+vork_surface_000000000002(values[2]);
}
"#,
                )],
                "main",
                &[("main", "compute")],
                &["-target", "spirv", "-emit-spirv-directly", "-O2"],
            )?
        };
        let output = compiler.localize_native_blends(&input)?;
        ensure!(
            output.len() < input.len(),
            "fixture did not localize native Blend state"
        );
        if let Ok(path) = std::env::var("VORK_BLEND_OUTPUT") {
            std::fs::write(path, output)?;
        }
        Ok(())
    }
}
