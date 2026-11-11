// Object-private storage for the connected compute dependency slice feeding a
// geometry stream. Pages are packed by the existing source page partition, so
// privatizing state does not increase a program's storage binding count.
struct GeometryScope {
    /// Draw splitting may give one authored object several render slots.
    objects: BTreeSet<u32>,
    node: usize,
    geometry: String,
    instance: Option<u32>,
    programs: BTreeSet<String>,
    outputs: BTreeSet<String>,
    buffers: ProgramBuffers,
    /// Local binding page to the canonical cursor/checkpoint page.
    pages: Vec<usize>,
}

pub(crate) fn geometry_solver_dependencies(
    scene: &v2::Scene,
    mut names: BTreeSet<String>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut programs = BTreeSet::new();
    loop {
        let before = programs.len();
        for (name, program) in scene.programs.iter().filter(|(_, program)| program.is_compute()) {
            if program.bindings.values().any(|name| names.contains(name)) {
                programs.insert(name.clone());
                names.extend(program.bindings.values().cloned());
            }
        }
        if programs.len() == before { break; }
    }
    let state = programs.iter().flat_map(|name| scene.programs[name].bindings.values().cloned()).collect();
    (programs, state)
}

enum GeometryPage { Private(u64), Shared(u32) }
struct GeometryBufferPlan {
    buffers:ProgramBuffers,
    pages:Vec<GeometryPage>,
    copies:Vec<(u32,u64,u32,u64,u64)>,
}
fn private_geometry_plan(source:&ProgramBuffers,state:&BTreeSet<String>,geometry:&BTreeSet<String>)->Result<GeometryBufferPlan> {
    let mut sizes=std::collections::BTreeMap::<u32,u64>::new();
    for name in state {
        let binding=&source.bindings[name];
        for span in &binding.spans {*sizes.entry(span.page).or_default()+=u64::from(span.count)*u64::from(binding.width)*4;}
    }
    let mut pages=Vec::new();let mut private=std::collections::BTreeMap::new();
    for (page,bytes) in sizes {
        let index=u32::try_from(pages.len()).context("private geometry page index overflow")?;
        pages.push(GeometryPage::Private(bytes));private.insert(page,(index,0u32));
    }
    let mut bindings=std::collections::BTreeMap::new();let mut shared=std::collections::BTreeMap::new();let mut copies=Vec::new();
    for name in state.union(geometry) {
        let mut binding=source.bindings[name].clone();
        for span in &mut binding.spans {
            if state.contains(name) {
                let (page,offset)=private.get_mut(&span.page).unwrap();
                let words=span.count.checked_mul(binding.width).context("private geometry span overflow")?;
                copies.push((span.page,u64::from(span.offset)*4,*page,u64::from(*offset)*4,u64::from(words)*4));
                span.page=*page;span.offset=*offset;*offset=offset.checked_add(words).context("private geometry page overflow")?;
            } else {
                let page=if let Some(page)=shared.get(&span.page) {*page} else {
                    let page=u32::try_from(pages.len()).context("shared geometry page index overflow")?;
                    pages.push(GeometryPage::Shared(span.page));shared.insert(span.page,page);page
                };
                span.page=page;
            }
        }
        bindings.insert(name.clone(),binding);
    }
    let initial_identity=source.initial_identity.iter().filter(|(name,_)|bindings.contains_key(*name)).map(|(name,identity)|(name.clone(),*identity)).collect();
    Ok(GeometryBufferPlan {buffers:ProgramBuffers {storage:Vec::new(),bindings,initial_identity},pages,copies})
}
fn private_geometry_buffers(r:&Renderer,source:&ProgramBuffers,state:&BTreeSet<String>,geometry:&BTreeSet<String>,pages:&mut Vec<wgpu::Buffer>,encoder:&mut wgpu::CommandEncoder)->Result<(ProgramBuffers,Vec<usize>)> {
    let mut plan=private_geometry_plan(source,state,geometry)?;let mut canonical=Vec::new();
    for page in &plan.pages {
        let buffer=match page {
            GeometryPage::Private(bytes)=> {
                let buffer=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:Some("object-private geometry solver state"),size:*bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
                canonical.push(pages.len());pages.push(buffer.clone());buffer
            }
            GeometryPage::Shared(page)=> {canonical.push(*page as usize);source.storage[*page as usize].clone()}
        };
        plan.buffers.storage.push(buffer);
    }
    for (source_page,source_offset,page,offset,bytes) in plan.copies {
        encoder.copy_buffer_to_buffer(&source.storage[source_page as usize],source_offset,&plan.buffers.storage[page as usize],offset,bytes);
    }
    Ok((plan.buffers,canonical))
}

// A nonspatial material reader can consume one object's geometry output
// directly. Multiple producers keep the existing global named namespace.
fn unique_geometry_source(scene:&v2::Scene,sources:&[Option<&v2::Node>],buffer:&str)->bool {
    let mut owners=sources.iter().flatten().filter(|node| {
        matches!(&node.kind,v2::NodeKind::Mesh {geometry,..}
            if scene.geometry[geometry].attributes.values().any(|name|name==buffer))
    });
    let Some(owner)=owners.next() else {return false;};
    owners.next().is_none() && matches!(&owner.kind,v2::NodeKind::Mesh {instances:None,..})
}

// The original namespace executes only where something outside object-local
// geometry consumes it. This avoids paying for an unused global solver in the
// usual geometry-only scene while preserving explicit named readers.
fn global_solver_consumers(scene: &v2::Scene, sources: &[Option<&v2::Node>]) -> BTreeSet<String> {
    let mut names: BTreeSet<_> = scene.buffers.iter().filter(|(_, buffer)| buffer.dims.is_some())
        .map(|(name, _)| name.clone()).collect();
    names.extend(scene.programs.values().filter(|program| !program.is_compute())
        .flat_map(|program| program.bindings.values().cloned()));
    for node in sources.iter().flatten() {
        if let v2::NodeKind::Mesh { instances: Some(instances), .. } = &node.kind {
            names.extend(instances.attributes.iter().filter(|(semantic,buffer)| {
                matches!(semantic.as_str(),"TRANSLATION"|"ROTATION"|"SCALE")
                    || !unique_geometry_source(scene,sources,buffer)
            }).map(|(_,buffer)|buffer.clone()));
            names.extend(instances.count.iter().cloned());
        }
    }
    geometry_solver_dependencies(scene, names).0
}

#[cfg(test)]
mod attribute_source_tests {
    use super::*;
    #[test]
    fn external_material_reader_preserves_geometry_producer_context() -> Result<()> {
        let mut scene:v2::Scene = v2::from_test_value(serde_json::from_str(include_str!("../tests/fixtures/computed-attribute.json"))?)?;
        let sources:Vec<_>=scene.nodes.iter().map(Some).collect();
        assert!(!global_solver_consumers(&scene,&sources).contains("copy_predecessor"),
            "a material reader must not erase the geometry producer's predecessor context");
        let mut other=scene.nodes[0].clone();
        other.name="second_producer".into();
        scene.nodes.push(other);
        let sources:Vec<_>=scene.nodes.iter().map(Some).collect();
        assert!(global_solver_consumers(&scene,&sources).contains("copy_predecessor"),
            "multiple geometry producers retain the global named namespace");
        Ok(())
    }
}
