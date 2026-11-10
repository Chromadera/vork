//! Native binding numbers follow wgpu-hal's sorted, dense Vulkan layout and
//! Metal's separate buffer/texture/sampler counters. Shared by build and runtime.
#[derive(Clone, Copy)]
pub enum Kind {
    Uniform,
    Read,
    Write,
    Sampler,
    Texture,
}
pub fn bindings(entry: &str) -> &'static [u32] {
    match entry {
        "resolve_color" | "resolve_fused" | "resolve_fused_single"
        | "post_source" => &[
            0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 24, 41,
            43, 44, 45, 46, 47, 51, 52, 53, 54, 55, 58, 67, 68, 69, 70,
        ],
        _ => panic!("unregistered resolve entry {entry}"),
    }
}
pub fn kind(binding: u32) -> Kind {
    match binding {
        5 => Kind::Uniform,
        11 | 43 | 44 | 45 | 46 | 67 => Kind::Read,
        4 | 10 | 47 | 70 => Kind::Write,
        12 => Kind::Sampler,
        _ => Kind::Texture,
    }
}
#[allow(dead_code)] // Used by build_slang.rs.
fn slot(kind: Kind, counters: &mut [u32; 3]) -> (char, u32) {
    let (class, counter) = match kind {
        Kind::Uniform => ('b', 0),
        Kind::Read => ('t', 0),
        Kind::Write => ('u', 0),
        Kind::Sampler => ('s', 2),
        Kind::Texture => ('t', 1),
    };
    let index = counters[counter];
    counters[counter] += 1;
    (class, index)
}
#[allow(dead_code)] // Used by build_slang.rs.
pub fn source(text: &str, entry: &str) -> String {
    let mut bindings = bindings(entry).to_vec();
    bindings.sort_unstable();
    let mut counters = [0; 3];
    let mut slots = Vec::new();
    for (dense, &binding) in bindings.iter().enumerate() {
        let (class, index) = slot(kind(binding), &mut counters);
        slots.push((binding, dense as u32, 0, class, index));
    }
    let output = counters[1];
    let mut result = String::new();
    let mut storage_textures = Vec::new();
    for line in text.lines() {
        if line.contains(",3)]]") {result.push_str(line);result.push('\n');continue;}
        if let Some(start) = line.find("[[vk::binding(") {
            let a = start + "[[vk::binding(".len();
            let b = a + line[a..].find(")]]").unwrap();
            let numbers: Vec<u32> = line[a..b]
                .split(',')
                .map(|s| s.trim().parse().unwrap())
                .collect();
            let (binding, group, class, index) = if numbers[1] == 1 {
                (0, 1, 'u', output)
            } else {
                let &(_, binding, group, class, index) =
                    slots.iter().find(|s| s.0 == numbers[0]).unwrap();
                (binding, group, class, index)
            };
            let mut declaration = format!(
                "{}[[vk::binding({binding},{group})]]{}",
                &line[..start],
                &line[b + 3..]
            );
            if let Some(a) = declaration.find(" : register(") {
                let b = a + declaration[a..].find(')').unwrap() + 1;
                declaration.replace_range(a..b, "");
            }
            let end = declaration
                .rfind(';')
                .expect("resolve resource declaration");
            // Vulkan's array declaration still requires a texture register class;
            // Metal wraps it in an argument buffer, whose slot is a buffer.
            let class = if class=='b' && declaration.contains("RWStructuredBuffer<") {'u'}
                else if class=='b' && (declaration.contains("Texture2DArray") || declaration.contains("StructuredBuffer<")) {'t'} else {class};
            declaration.insert_str(end, &format!(" : register({class}{index})"));
            if declaration.contains("RWTexture2D<") {
                storage_textures.push((index,declaration));
                continue;
            }
            result.push_str(&declaration);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    // Slang's Metal backend allocates storage textures from the free texture
    // slots in declaration order, despite their u-register semantics. Read-only
    // t-registers reserve the other slots, so order writable textures explicitly.
    storage_textures.sort_by_key(|(index,_)| *index);
    let mut source=String::new();
    for (_,declaration) in storage_textures {source.push_str(&declaration);source.push('\n');}
    source.push_str(&result);
    if entry == "resolve_fused" || entry == "resolve_fused_single" {source.insert_str(0,"#define VORK_RESOLVE_FUSED\n");}
    if entry == "resolve_fused_single" {source.insert_str(0,"#define VORK_RESOLVE_SINGLE\n");}
    source
}
