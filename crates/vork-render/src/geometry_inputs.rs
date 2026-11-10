// Generated read-only stage inputs. Previous-state buffers are derived inputs,
// recopied before the entire scheduled tick; they are not checkpoint state.
#[derive(Clone, Copy)]
enum GeometryInput {
    Predecessor(u32),
    Previous(u32),
    Reference,
}
struct GeometryInputBinding {
    binding: u32,
    input: GeometryInput,
}

pub(crate) fn geometry_has_deformation(scene: &v2::Scene, geometry: &v2::Geometry) -> bool {
    if !geometry.morphs.is_empty() { return true; }
    let Some((name,_)) = scene.geometry.iter().find(|(_,candidate)| std::ptr::eq(*candidate,geometry)) else { return false; };
    fn visit(nodes: &[v2::Node], name: &str) -> bool {
        nodes.iter().any(|node| matches!(&node.kind, v2::NodeKind::Mesh {geometry,skin:Some(_),..} if geometry==name)
            || visit(&node.children,name))
    }
    visit(&scene.nodes,name)
}

pub(crate) struct PredecessorPlan {
    pub buffers: ProgramBuffers,
    pub sizes: Vec<u64>,
    pub copies: Vec<(u32,u64,u32,u64,u64)>,
}

/// Reuse the geometry namespace packer for read-only columns. No unrelated
/// simulation ranges, leading page gaps or canonical checkpoint state are kept.
pub(crate) fn predecessor_plan(named: &ProgramBuffers, geometry: &v2::Geometry) -> Result<PredecessorPlan> {
    let names = geometry.attributes.values().chain(geometry.widths.iter())
        .chain(geometry.count.iter()).chain(geometry.ids.iter()).cloned().collect();
    let plan = private_geometry_plan(named,&names,&names)?;
    let sizes = plan.pages.into_iter().map(|page| match page {
        GeometryPage::Private(bytes) => bytes,
        GeometryPage::Shared(_) => unreachable!("all predecessor columns are packed"),
    }).collect();
    Ok(PredecessorPlan {buffers:plan.buffers,sizes,copies:plan.copies})
}
#[derive(Default)]
struct PreviousState {
    pages: std::collections::BTreeMap<usize, wgpu::Buffer>,
    ranges: Vec<(ComputeSchedule, usize, u64, u64)>,
}
impl PreviousState {
    fn bind(&mut self, r: &Renderer, pages: &[wgpu::Buffer], page: usize) -> &wgpu::Buffer {
        self.pages.entry(page).or_insert_with(|| crate::gpu_memory::buffer(&r.device, &wgpu::BufferDescriptor {
            label: Some("read-only previous geometry stage state"), size: pages[page].size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        }))
    }
    fn capture(&self, encoder: &mut wgpu::CommandEncoder, pages: &[wgpu::Buffer], schedule: ComputeSchedule) {
        for &(role, page, offset, bytes) in &self.ranges {
            if role == schedule {
                encoder.copy_buffer_to_buffer(&pages[page], offset, &self.pages[&page], offset, bytes);
            }
        }
    }
}
fn readonly_accessor(
    header: &mut String, name: &str, layout: &AttributeBinding,
    kind: v2::BufferType, prefix: &str,
) -> Result<()> {
    layout.count.checked_mul(layout.width).context("geometry input exceeds u32 word addressing")?;
    writeln!(header, "static const uint {name}_count = {}u; uint _vork_{name}_load(uint word) {{", layout.count)?;
    for span in &layout.spans {
        let first = span.first * layout.width;
        let end = (span.first + span.count) * layout.width;
        writeln!(header, "if(word >= {first}u && word < {end}u) return {prefix}_{}[{}u+word-{first}u];", span.page, span.offset)?;
    }
    header.push_str("return 0u; }\n");
    typed_element_accessors(header, name, kind, false)
}
fn geometry_input_header(
    header: &mut String, scene: &v2::Scene, geometry: Option<&v2::Geometry>, packed: bool,
    named: &ProgramBuffers, state: &ProgramBuffers, program: &v2::Program,
    used: &BTreeSet<u32>, first_binding: u32,
) -> Result<Vec<GeometryInputBinding>> {
    let mut bindings = Vec::new();
    for &page in used {
        let binding = first_binding + bindings.len() as u32;
        writeln!(header, "[[vk::binding({binding},0)]] StructuredBuffer<uint> vork_previous_page_{page};")?;
        bindings.push(GeometryInputBinding { binding, input: GeometryInput::Previous(page) });
    }
    for (alias, buffer) in &program.bindings {
        readonly_accessor(header, &format!("previous_{alias}"), &state.bindings[buffer], scene.buffers[buffer].element_type, "vork_previous_page")?;
    }
    if let Some(geometry) = geometry {
        let packed = (packed || geometry_has_deformation(scene,geometry)).then(|| predecessor_plan(named,geometry)).transpose()?;
        let named = packed.as_ref().map_or(named,|plan| &plan.buffers);
        let names: BTreeSet<_> = geometry.attributes.values().chain(geometry.widths.iter())
            .chain(geometry.count.iter()).chain(geometry.ids.iter()).collect();
        let pages: BTreeSet<_> = names.iter().flat_map(|name| named.bindings[*name].spans.iter().map(|span| span.page)).collect();
        for page in pages {
            let binding = first_binding + bindings.len() as u32;
            writeln!(header, "[[vk::binding({binding},0)]] StructuredBuffer<uint> vork_geometry_page_{page};")?;
            bindings.push(GeometryInputBinding { binding, input: GeometryInput::Predecessor(page) });
        }
        writeln!(header, "static const uint geometry_attribute_count = {}u;", geometry.attributes.len())?;
        for (index, (semantic, buffer)) in geometry.attributes.iter().enumerate() {
            let name = format!("geometry_attribute_{index}");
            readonly_accessor(header, &name, &named.bindings[buffer], scene.buffers[buffer].element_type, "vork_geometry_page")?;
            // Semantic aliases are reserved for the built-in glTF channels.
            // Numbered accessors retain unrestricted custom attribute names.
            if matches!(semantic.as_str(), "POSITION" | "NORMAL" | "TANGENT")
                || ["TEXCOORD_", "COLOR_", "JOINTS_", "WEIGHTS_"].iter().any(|prefix|
                    semantic.strip_prefix(*prefix).is_some_and(|index| !index.is_empty() && index.bytes().all(|c| c.is_ascii_digit()))) {
                readonly_accessor(header, &format!("geometry_{semantic}"), &named.bindings[buffer], scene.buffers[buffer].element_type, "vork_geometry_page")?;
            }
        }
        for (alias, buffer) in [("width", &geometry.widths), ("id", &geometry.ids), ("count", &geometry.count)] {
            if let Some(buffer) = buffer {
                readonly_accessor(header, &format!("geometry_{alias}"), &named.bindings[buffer], scene.buffers[buffer].element_type, "vork_geometry_page")?;
            }
        }
        let capacity = named.bindings[&geometry.attributes["POSITION"]].count;
        writeln!(header, "static const uint geometry_vertex_capacity = {capacity}u;")?;
        writeln!(header, "uint geometry_vertex_count() {{ return {}; }}",
            if geometry.count.is_some() {format!("min(geometry_count_load(0u),{capacity}u)")} else {format!("{capacity}u")})?;
        writeln!(header, "uint geometry_source_vertex(uint i) {{return i<{capacity}u ? i : 0xffffffffu;}}")?;
    }
    let referenced=scene.materials.values().filter(|material|scene.programs.get(&material.program).is_some_and(|candidate|std::ptr::eq(candidate,program)))
        .any(|material|material.inputs.values().any(|input|matches!(input,v2::Input::Geometry(_))));
    if referenced {
        let binding=first_binding+bindings.len() as u32;
        header.push_str(&crate::geometry_reference::header(binding,crate::geometry_reference::names(scene).len()));
        bindings.push(GeometryInputBinding {binding,input:GeometryInput::Reference});
    }
    Ok(bindings)
}

// Inspect the compiled entry, including calls through imported helpers. An
// unused implicit input adds neither a binding nor a state copy to old scenes.
fn used_compute_bindings(source: &str) -> Result<BTreeSet<u32>> {
    vork_slang::compute_used_bindings(source)
}
