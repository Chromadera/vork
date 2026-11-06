//! Native binding numbers follow wgpu-hal's sorted, dense Vulkan layout and
//! Metal's separate buffer/texture/sampler counters. Shared by build and runtime.
#[derive(Clone, Copy)]
pub enum Kind {
    Uniform,
    ArgumentTable,
    Read,
    Write,
    Sampler,
    Texture,
    Storage,
    Acceleration,
}
pub fn bindings(entry: &str) -> &'static [u32] {
    match entry {
        "volume_receiver_support" | "volume_apply" | "volume_lighting" | "volume_visibility_world" | "resolve_color" | "resolve_fused" | "resolve_fused_single"
        | "post_source" => &[
            0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 24, 41,
            43, 44, 45, 46, 47, 51, 52, 53, 54, 55, 58, 67, 68, 69, 70,
        ],
        _ => panic!("unregistered resolve entry {entry}"),
    }
}
/// Volume transport and receiver-sample pages bind as two arrays in their own
/// group of the volume passes, whatever the page count: a binding per page
/// exhausted Metal's 31 buffer slots per stage, and wgpu forbids binding arrays
/// beside the resolve group's uniforms. Group 3 holds hit buffers when installed.
pub const PAGE_GROUP: u32 = 4;
pub const FROXEL_PAGES: u32 = 0;
pub const SAMPLE_PAGES: u32 = 1;
/// Metal buffer slot of the first page table: after the volume pass's resolve,
/// material and shadow groups and, when installed, its hit table.
pub fn page_base(entry: &str, layout: Layout) -> u32 {
    hit_base(entry, layout) + u32::from(layout.shadow_closures)
}
pub fn kind(binding: u32) -> Kind {
    match binding {
        3 | 5 | 39 | 64 => Kind::Uniform,
        56 | 63 => Kind::ArgumentTable,
        11 | 21 | 38 | 43 | 44 | 45 | 46 | 60 | 65 | 66 | 67 | 71 => Kind::Read,
        72.. => Kind::Write,
        4 | 10 | 17 | 19 | 22 | 23 | 47 | 57 | 61 | 70 => Kind::Write,
        12 => Kind::Sampler,
        26 | 27 | 30 | 31 | 32 | 33 | 36 | 37 => Kind::Storage,
        59 => Kind::Acceleration,
        _ => Kind::Texture,
    }
}
fn slot(kind: Kind, counters: &mut [u32; 3]) -> (char, u32) {
    let (class, counter) = match kind {
        Kind::Uniform | Kind::ArgumentTable => ('b', 0),
        Kind::Read => ('t', 0),
        Kind::Write => ('u', 0),
        Kind::Sampler => ('s', 2),
        Kind::Texture => ('t', 1),
        Kind::Acceleration => ('t',0),
        Kind::Storage => ('u', 1),
    };
    let index = counters[counter];
    counters[counter] += 1;
    (class, index)
}
/// Resource numbering follows the installed layout, including bindings unused
/// by this entry. Eliminating shader code must never shift a later descriptor.
#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub volume: bool,
    pub rays: bool,
    pub shadow_closures: bool,
}
/// BRDF and layer tables read by straight-segment surface closures.
pub const SHADOW_CLOSURE_BINDINGS: [u32; 2] = [13,18];
const SHADOW_BINDINGS: [u32; 1] = [66];
/// First group-1 buffer, after the complete retained resolve group-0 layout.
pub fn material_base(entry: &str, layout: Layout) -> u32 {
    let mut bindings = bindings(entry).to_vec();
    if layout.volume {
        bindings.extend([3,64,65]);
    }
    if layout.rays {bindings.extend([59,60,71]);}
    if layout.shadow_closures {bindings.extend(SHADOW_CLOSURE_BINDINGS);}
    bindings.iter().filter(|&&b| matches!(kind(b),Kind::Uniform|Kind::Read|Kind::Write|Kind::Acceleration)).count() as u32
}
/// Group 3 follows resolve resources, the material table, and the shadow tables.
pub fn hit_base(entry: &str, layout: Layout) -> u32 {
    let mut counters = [material_base(entry, layout) + crate::binding_abi::MATERIAL.len() as u32, 0, 0];
    for binding in SHADOW_BINDINGS { slot(kind(binding), &mut counters); }
    counters[0]
}
pub fn source(text: &str, entry: &str, layout: Layout) -> String {
    let mut bindings = bindings(entry).to_vec();
    if layout.volume {
        bindings.extend([3,64,65,66]);
    }
    if layout.rays {bindings.extend([59,60,71]);}
    if layout.shadow_closures {bindings.extend(SHADOW_CLOSURE_BINDINGS);}
    bindings.sort_unstable();
    let shadow_bindings = SHADOW_BINDINGS;
    let mut counters = [0; 3];
    let mut slots = Vec::new();
    for (dense, &binding) in bindings.iter().filter(|b| !shadow_bindings.contains(b)).enumerate() {
        let (class, index) = slot(kind(binding), &mut counters);
        slots.push((binding, dense as u32, 0, class, index));
    }
    // Each volume pass's group 1 is the material table (records, words and two
    // argument buffers plus the shader error word); group 2 follows those slots.
    if layout.volume && matches!(entry,"volume_receiver_support"|"volume_lighting" | "volume_visibility_world"|"volume_apply") { counters[0] += crate::binding_abi::MATERIAL.len() as u32; }
    for (dense, &binding) in shadow_bindings.iter().enumerate() {
        if bindings.contains(&binding) {
            let (class,index) = slot(kind(binding), &mut counters);
            slots.push((binding,dense as u32,2,class,index));
        }
    }
    let output = counters[1];
    counters[1] += 1;
    // Keep globals used only by other entries semantically available, assigning
    // distinct slots. Dead resources disappear before native code emission.
    for binding in 0..=71 {
        if !bindings.contains(&binding) {
            let (class, index) = slot(kind(binding), &mut counters);
            slots.push((binding, binding, 2, class, index));
        }
    }
    let mut result = String::new();
    let mut storage_textures = Vec::new();
    for line in text.lines() {
        if line.contains(",3)]]") || line.contains(&format!(",{PAGE_GROUP})]]")) {result.push_str(line);result.push('\n');continue;}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_volume_ray_resources_preserve_late_resolve_bindings() {
        // The shader need not read light sampling or enable ray tracing. The
        // installed group still reserves its TLAS and alpha-table descriptors.
        let text = "[[vk::binding(67,0)]] StructuredBuffer<uint> sampling;";
        for entry in ["resolve_color", "post_source"] {
            let base=bindings(entry).iter().position(|&binding|binding==67).unwrap();
            for (layout, first) in [
                (Layout::default(), base),
                (Layout {volume: true, rays: false, ..Default::default()}, base+3),
                (Layout {volume: true, rays: true, ..Default::default()}, base+5),
            ] {
                let source = source(text, entry, layout);
                assert!(source.contains(&format!("[[vk::binding({first},0)]]")),
                    "{entry}: late binding shifted: {source}");
            }
        }
    }

    #[test]
    fn page_tables_follow_every_installed_volume_group() {
        // Page tables keep their own group; group 0 numbering is unchanged.
        let declarations = "[[vk::binding(0,4)]] ParameterBlock<Pages> pages : register(VORK_FROXEL_PAGES_REGISTER);";
        for coverage in [false, true] {
            let layout = Layout {volume:true, rays:true, shadow_closures:coverage};
            for entry in ["volume_receiver_support","volume_visibility_world","volume_lighting","volume_apply"] {
                assert!(source(declarations,entry,layout).contains(declarations));
                // Metal: group 0, the material table, shadow tables, then hits.
                assert_eq!(page_base(entry,layout), hit_base(entry,layout)+u32::from(coverage));
            }
        }
    }
}
