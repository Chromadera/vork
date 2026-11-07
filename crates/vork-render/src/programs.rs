//! Slang material programs (docs/PROGRAMS.md). Each distinct program compiles
//! once through the shared disk cache, retaining native modules for raster stages.
use crate::{Renderer, disk_cache};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
};
use vork_scene::v2;
use vork_slang::{Abi, Compiler, Program};

pub(crate) fn program_source(
    base: &Path,
    name: &str,
    program: &v2::Program,
) -> Result<(String, PathBuf)> {
    match (&program.file, &program.source) {
        (Some(file), None) => {
            let path = base.join(file);
            Ok((
                std::fs::read_to_string(&path)
                    .with_context(|| format!("program {name}: read {}", path.display()))?,
                path,
            ))
        }
        (None, Some(source)) => Ok((source.clone(), base.join(program.source_origin.as_deref().map(PathBuf::from).unwrap_or_else(||PathBuf::from(format!("programs.{name}")))))),
        _ => anyhow::bail!("program {name}: exactly one of file/source required"),
    }
}
// Bundled and custom materials keep the same closure semantics across integrators.
fn material_source(scene: &v2::Scene, base: &Path, name: &str) -> Result<(String, PathBuf)> {
    if let Some(source) = vork_slang::module_source(name) {
        // Bundled modules own their imports. A scene's directory must not
        // multiply their reflection/native tags or invalidate setup's artifacts.
        Ok((source.to_owned(), PathBuf::from(format!("{name}.slang"))))
    } else { program_source(base, name, &scene.programs[name]) }
}

/// ShadingContext as far as shading provides it, and every OpenPBR Closure
/// field (the table holds only fields this renderer shades). Programs also set
/// the coat's shading normal: every surface stage shades `geometry_coat_normal`
/// (unset, it stays the unperturbed normal); std/openpbr has no input for it.
pub(crate) fn abi() -> Abi {
    Abi { aov_count: 0, context: Abi::spec_context(), closure: vork_slang::openpbr_fields() }
}

/// A material's program instance: module source (inline or a file relative to
/// the scene) and its supplied inputs, checked against reflection at compile.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Instance {
    pub name: String,
    pub file: Option<PathBuf>,
    pub source: Option<String>,
    pub inputs: std::collections::BTreeMap<String, serde_json::Value>,
    /// Image inputs, resolved from the scene's image table.
    pub images: Vec<Image>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Image {
    pub input: String,
    pub source: crate::texture_source::TextureSource,
    pub srgb: bool,
    pub sampler: v2::Sampler,
}
/// The program instance of a non-OpenPBR material; None for std/openpbr, whose
/// records are installed natively (materials::install_openpbr).
pub(crate) fn instance(scene: &v2::Scene, name: &str) -> Result<Option<Instance>> {
    let material = scene
        .materials
        .get(name)
        .with_context(|| format!("unknown material {name}"))?;
    if material.program == "std/openpbr" {
        return Ok(None);
    }
    let program = scene.programs.get(&material.program);
    ensure!(
        program.is_some() || vork_slang::module_source(&material.program).is_some(),
        "material {name}: unknown program {}",
        material.program
    );
    let mut images = Vec::new();
    for (input_name, value) in &material.inputs {
        let image_name = match value {
            v2::Input::Texture(binding) => &binding.image,
            _ => continue,
        };
        let image = scene
            .images
            .get(image_name)
            .with_context(|| {
                format!(
                    "material {name}: input {input_name}: {}",
                    v2::diagnostics::unknown("image", image_name, scene.images.keys().map(String::as_str))
                )
            })?;
        images.push(Image {
            input: input_name.clone(),
            source: crate::texture_source::TextureSource::scene(image_name, image),
            srgb: image.is_srgb(),
            sampler: image.sampler(),
        });
    }
    Ok(Some(Instance {
        name: material.program.clone(),
        file: program.and_then(|p| p.file.as_ref()).map(Into::into),
        source: program.and_then(|p| p.source.clone()),
        inputs: material
            .inputs
            .iter()
            .map(|(name, value)| Ok((name.clone(), serde_json::to_value(value)?)))
            .collect::<Result<_>>()?,
        images,
    }))
}

pub(crate) struct Programs {
    /// Program and attribute identity for the installed pipeline cache.
    pub(crate) identity: String,
    /// The same programs as Slang modules for the native colour passes; None
    /// when the scene has only named attributes.
    pub(crate) native: Option<std::sync::Arc<Native>>,
    compiled: Vec<Program>,
    /// Capabilities proved from each instance’s material words.
    bound: HashMap<String, vork_slang::ProgramCapabilities>,
    /// Per instance of a program that may produce coverage: whether that
    /// coverage can change between frames (EffectProbe::coverage_varies).
    coverage_varies: HashMap<String, bool>,
    physical_hair: bool,
    aovs: std::collections::BTreeMap<String,u32>,
    /// Compiled program index per instance.
    instances: HashMap<String, usize>,
}
impl Programs {
    pub(crate) fn input_layout(&self, id: u32) -> &vork_slang::InputLayout {
        &self.compiled[id as usize - 1].inputs
    }
    pub(crate) fn dependencies(&self) -> vork_slang::ProgramDependencies {
        self.compiled.iter().fold(vork_slang::ProgramDependencies::default(), |mut d, p| {
            d.include(p.capabilities.dependencies);
            d
        })
    }

    pub(crate) fn program_dependencies(&self) -> Vec<vork_slang::ProgramDependencies> {
        self.compiled.iter().map(|p| p.capabilities.dependencies).collect()
    }

    pub(crate) fn has_lit(&self)->bool {self.compiled.iter().any(|p|p.capabilities.lit)}
    pub(crate) fn has_displacement(&self)->bool {self.compiled.iter().any(|p|p.capabilities.displacement)}
    pub(crate) fn displacement_timed(&self,id:u32)->bool {self.compiled[id as usize-1].displacement_uses_time}
    pub(crate) fn displacement_uses_normal(&self,id:u32)->bool {self.compiled[id as usize-1].displacement_uses_normal}
    pub(crate) fn displacement_descriptors(&self, id:u32, base:usize, words:&[u32])->Vec<(u32,Vec<u32>)> {
        let program=&self.compiled[id as usize-1];
        program.displacement_attributes.iter().map(|name| {
            let field=program.inputs.fields.iter().find(|f|&f.name==name).unwrap();
            let vork_slang::Type::Attribute(width)=field.ty else {unreachable!()};
            let descriptor=base+words[base+field.offset] as usize;
            (width,words[descriptor..descriptor+5].to_vec())
        }).collect()
    }
    pub(crate) fn displaces(&self, id: u32) -> bool { id.checked_sub(1).and_then(|i| self.compiled.get(i as usize)).is_some_and(|p| p.capabilities.displacement) }
    pub(crate) fn attribute_width(&self,p:&Instance,input:&str)->Option<u32> {
        self.compiled[self.instances[&instance_key(p)]].inputs.fields.iter().find_map(|f|if f.name==input {if let vork_slang::Type::Attribute(n)=f.ty {Some(n)} else {None}} else {None})
    }
    pub(crate) fn is_hair(&self, p: &Instance) -> bool { self.compiled[self.instances[&instance_key(p)]].capabilities.hair }
    pub(crate) fn has_physical_hair(&self) -> bool { self.physical_hair }
    pub(crate) fn is_physical_hair(&self, p: &Instance) -> bool { self.bound.get(&instance_key(p)).is_some_and(|c| c.physical_hair) }
    /// Admission uses the same bound proof as launch normalization. Keep the
    /// program-wide capability unchanged for shader generation.
    pub(crate) fn is_subsurface(&self, p: &Instance) -> bool {
        let key = instance_key(p);
        self.bound.get(&key).map_or(self.compiled[self.instances[&key]].capabilities.subsurface, |c| c.subsurface)
    }
    /// The instance's coverage can change between frames on an object that
    /// does not move: never without program opacity; unproven is true.
    pub(crate) fn coverage_varies(&self, p: &Instance) -> bool {
        let key = instance_key(p);
        self.compiled[self.instances[&key]].capabilities.opacity && self.coverage_varies.get(&key).copied().unwrap_or(true)
    }
    /// The program's capabilities, with opacity and transmission narrowed to
    /// what this instance's material words can produce. Solid scattering also
    /// narrows to the instance's thin-walled state; coverage picks the
    /// material's route. Other effects keep the program's value.
    pub(crate) fn capabilities(&self, p: &Instance) -> vork_slang::ProgramCapabilities {
        let key = instance_key(p);
        let mut capabilities = self.compiled[self.instances[&key]].capabilities;
        if let Some(bound) = self.bound.get(&key) {
            capabilities.opacity &= bound.opacity;
            capabilities.transmission &= bound.transmission;
            capabilities.spatial_subsurface &= bound.spatial_subsurface;
            capabilities.emission &= bound.emission;
            capabilities.anisotropy &= bound.anisotropy;
            capabilities.film &= bound.film;
            capabilities.coat &= bound.coat;
            capabilities.fuzz &= bound.fuzz;
            capabilities.tint &= bound.tint;
        }
        capabilities
    }
    /// Program id (1-based dispatch index) and an instance's input words; image
    /// inputs pack the scene material table word of their image.
    pub(crate) fn pack(
        &self,
        p: &Instance,
        image_words: &std::collections::BTreeMap<String, u64>,
        scene: &v2::Scene,
        base: &Path,
    ) -> Result<(u32, Vec<u32>)> {
        let index = self.instances[&instance_key(p)];
        let mut words = pack_inputs(&self.compiled[index].inputs, &p.inputs, scene, base, |name, value| {
                if self.compiled[index].inputs.fields.iter().any(|f| f.name == name && f.ty == vork_slang::Type::Aov) {
                    let name=value.get("aov").and_then(serde_json::Value::as_str).with_context(|| format!("input {name}: Aov inputs take {{\"aov\": name}} naming a render.aovs entry, not {value}"))?;
                    return self.aovs.get(name).copied().with_context(|| format!("input {name}: {}", v2::diagnostics::unknown("render.aovs entry", name, self.aovs.keys().map(String::as_str))));
                }
                image_words
                    .get(name)
                    .map(|&word|word as u32)
                    .with_context(|| format!("input {name}: Image inputs take {{\"image\": name}}, not {value}"))
            })
            .with_context(|| format!("program {}", p.name))?;
        for field in &self.compiled[index].inputs.fields {
            if field.ty == vork_slang::Type::Image {
                let Some(&descriptor)=image_words.get(&field.name) else {continue;};
                words[field.offset]=u32::try_from(words.len())?;
                words.extend([descriptor as u32,(descriptor>>32) as u32]);
            }
        }
        Ok((index as u32 + 1, words))
    }
}
/// All stages of a material use the same numeric/grid input layout and relocation.
/// Each stage supplies the resource kinds it supports beyond named grids.
pub(crate) fn pack_inputs(
    layout: &vork_slang::InputLayout,
    values: &std::collections::BTreeMap<String, serde_json::Value>,
    scene: &v2::Scene,
    base: &Path,
    mut resource: impl FnMut(&str, &serde_json::Value) -> Result<u32>,
) -> Result<Vec<u32>> {
    let mut grids = Vec::new();
    let mut words = layout.pack(values, |name, value| {
        let field = layout.fields.iter().find(|f| f.name == name).context("program resource field")?;
        let expected = match field.ty {
            vork_slang::Type::FloatGrid => v2::BufferType::F32,
            vork_slang::Type::VectorGrid => v2::BufferType::Vec3,
            vork_slang::Type::Attribute(width) => {
                let binding: v2::AttributeBinding=serde_json::from_value(value.clone()).with_context(||format!("input {name}: Attribute inputs require {{attribute, channel?, factor?}}"))?;
                let names=crate::attributes::names(scene)?;
                let slot=names.iter().position(|n|n==&binding.attribute).context("attribute binding not installed")?;
                let channel=crate::attributes::channel(&binding,width)?;
                let factors=crate::attributes::factor(&binding,width)?;
                let offset=u32::try_from(layout.words+grids.len())?;
                grids.push(slot as u32 | channel<<8);
                grids.extend(factors.map(f32::to_bits));
                return Ok(offset);
            }
            _ => return resource(name, value),
        };
        let buffer_name = value.get("grid").and_then(serde_json::Value::as_str)
            .with_context(|| format!("input {name}: grid inputs take {{\"grid\": buffer-name}}, not {value}"))?;
        let buffer = scene.buffers.get(buffer_name).with_context(|| format!("unknown grid buffer {buffer_name}"))?;
        ensure!(buffer.element_type == expected, "grid input {name}: voxel type mismatch");
        if let Some(dims) = buffer.dims {
            let data = crate::buffers::AttributeData::decode(buffer, base)?;
            let offset = u32::try_from(layout.words + grids.len())?;
            // Dense descriptor: sentinel, dimensions, then cell-centred words.
            let (lo,hi) = data.words.iter().map(|&w| f32::from_bits(w)).fold((0f32,0f32), |(lo,hi),v| (lo.min(v),hi.max(v)));
            grids.extend([u32::MAX, dims[0], dims[1], dims[2], lo.to_bits(), hi.to_bits()]);
            grids.extend(data.words);
            return Ok(offset);
        }
        let grid = crate::vdb::Grid::load(buffer, base)?;
        let offset = u32::try_from(layout.words + grids.len())?;
        grids.push(u32::try_from(grid.words.len())?);
        grids.extend(grid.words);
        Ok(offset)
    })?;
    for field in &layout.fields {
        if matches!(field.ty, vork_slang::Type::FloatGrid | vork_slang::Type::VectorGrid) {
            ensure!(values.contains_key(&field.name), "grid input {} must name a buffer", field.name);
        }
    }
    words.extend(grids);
    Ok(words)
}

fn instance_key(p: &Instance) -> String {
    serde_json::to_string(p).expect("program instance serializes")
}

/// Resolve surface/geometry combinations before compiling their GPU consumers.
/// Copies are per node, so a shared material keeps its supported uses unchanged.
pub(crate) fn normalize_launch_bindings(r:&Renderer, scene:&mut v2::Scene, base:&Path)->Result<()> {
    let mut replacements=std::collections::BTreeMap::<String,(v2::Material,String)>::new();
    let mut nodes=scene.nodes.clone();
    let mut path_needed=false;
    fn computed_placements(scene:&v2::Scene,nodes:&[v2::Node])->bool {
        nodes.iter().any(|node|crate::compute::places_instances(scene,node) || computed_placements(scene,&node.children))
    }
    // Fibre captures fit the whole scene, including ordinary computed blockers
    // on other nodes. A same-node test misses those cross-node dependencies.
    let computed=computed_placements(scene,&scene.nodes);
    let compiler=Compiler::discover();
    let mut abi=abi(); abi.aov_count=scene.render.aovs.len();
    let mut proof=std::collections::BTreeMap::new();
    fn visit(r:&Renderer,scene:&v2::Scene,nodes:&mut [v2::Node],base:&Path,compiler:&Compiler,abi:&Abi,
        proof:&mut std::collections::BTreeMap<String,vork_slang::ProgramCapabilities>,replacements:&mut std::collections::BTreeMap<String,(v2::Material,String)>,path_needed:&mut bool,computed:bool)->Result<()> {
        for node in nodes {
            if let v2::NodeKind::Light {shape:v2::LightShape::Environment,material:Some(name),..}=&node.kind {
                if scene.materials[name].inputs.values().any(|v|matches!(v,v2::Input::Texture(binding) if scene.images[&binding.image].render.is_some())) {
                    crate::warnings::dropped("environment_render_image", serde_json::json!({"node":node,"material":scene.materials[name]}), format!("warning: node {:?}: ignored environment material {name} sampling a render image; using the light's colour",node.name));
                    if let v2::NodeKind::Light {material,..}=&mut node.kind {*material=None;}
                }
            }
            let (bindings,curves,implicit):(Vec<&mut String>,bool,bool)=match &mut node.kind {
                v2::NodeKind::Mesh {materials,geometry,..}=>(materials.iter_mut().collect(),matches!(scene.geometry[geometry].topology,v2::Topology::Curves),matches!(scene.geometry[geometry].topology,v2::Topology::Implicit)),
                v2::NodeKind::Asset {materials,..}=>(materials.values_mut().collect(),false,false),
                _=>(Vec::new(),false,false),
            };
            for binding in bindings {
                let material=&scene.materials[binding.as_str()];
                let mut replacement=None;
                if material.program=="std/openpbr" {
                    if implicit && (material.inputs.get("subsurface_weight").is_some_and(|v|!matches!(v,v2::Input::Number(x) if *x==0.)) || scene.animation.iter().flat_map(|l|l.tracks.iter().flatten()).any(|t|t.path==["materials",binding.as_str(),"inputs","subsurface_weight"])) {
                        let mut m=material.clone();m.inputs.insert("subsurface_weight".into(),v2::Input::Number(0.));
                        crate::warnings::dropped("implicit_subsurface", serde_json::json!({"node":node.name,"material":binding,"inputs":material.inputs,"tracks":scene.animation.iter().flat_map(|layer|layer.tracks.iter().flatten()).filter(|track|matches!(track.path.as_slice(),[root,target,..] if root=="materials" && target.as_str()==binding.as_str())).collect::<Vec<_>>()}), format!("warning: node {:?}, material {}: ignored subsurface scattering on implicit geometry",node.name,binding));
                        replacement=Some(m);
                    }
                } else if !curves || implicit || computed {
                    let capability=if let Some(c)=proof.get(binding.as_str()) {*c} else {
                        let (source,origin)=material_source(scene,base,&material.program)?;
                        let program=compile(r,compiler,abi,&source,&origin)?;
                        let dynamic=scene.animation.iter().flat_map(|l|l.tracks.iter().flatten()).filter_map(|t|match t.path.as_slice(){[root,name,inputs,field] if root=="materials" && name.as_str()==binding.as_str() && inputs=="inputs"=>Some(field.clone()),_=>None}).collect();
                        let values=material.inputs.iter().map(|(k,v)|Ok((k.clone(),serde_json::to_value(v)?))).collect::<Result<std::collections::BTreeMap<_,_>>>()?;
                        let words=program.inputs.effect_words(&values,&dynamic)?;
                        let c=program.effect_probe()?.capabilities(&words);proof.insert(binding.clone(),c);c
                    };
                    if curves && computed && capability.physical_hair {*path_needed=true;}
                    if (capability.hair && !curves) || (implicit && capability.subsurface) {
                        crate::warnings::dropped("surface_geometry", serde_json::json!({"node":node.name,"material":binding,"inputs":material.inputs,"program":material.program,"tracks":scene.animation.iter().flat_map(|layer|layer.tracks.iter().flatten()).filter(|track|matches!(track.path.as_slice(),[root,target,..] if root=="materials" && target.as_str()==binding.as_str())).collect::<Vec<_>>()}), format!("warning: node {:?}, material {}: ignored unsupported {} surface on this geometry; using default OpenPBR surface",node.name,binding,if capability.hair {"Hair"} else {"custom subsurface"}));
                        replacement=Some(serde_json::from_value(serde_json::json!({}))?);
                    }
                }
                if let Some(m)=replacement {
                    let name=format!("__vork_launch_{}",replacements.len());replacements.insert(name.clone(),(m,binding.clone()));*binding=name;
                }
            }
            visit(r,scene,&mut node.children,base,compiler,abi,proof,replacements,path_needed,computed)?;
        }
        Ok(())
    }
    visit(r,scene,&mut nodes,base,&compiler,&abi,&mut proof,&mut replacements,&mut path_needed,computed)?;
    scene.nodes=nodes;
    for layer in &mut scene.animation {
        if let Some(tracks)=&mut layer.tracks {
            let mut copies=Vec::new();
            for (name,(material,original)) in &replacements {
                if material.program=="std/openpbr" && scene.materials[original].program=="std/openpbr" {
                    for track in tracks.iter().filter(|t|matches!(t.path.as_slice(),[root,target,inputs,field] if root=="materials" && target==original && inputs=="inputs" && field!="subsurface_weight")) {
                        let mut copy=track.clone();copy.path[1]=name.clone();copies.push(copy);
                    }
                }
            }
            tracks.extend(copies);
        }
    }
    scene.materials.extend(replacements.into_iter().map(|(name,(material,_))|(name,material)));
    if path_needed && !scene.render.settings.integrator.is_path() {
        eprintln!("warning: physical Hair with computed placement uses std/path transport");
        crate::rt::promote_indirect_to_path(scene);
    }
    Ok(())
}

/// Independently routed stages of materials actually bound to meshes/assets.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Stages {
    pub surface: bool,
    pub medium: bool,
    pub displacement: bool,
}
pub(crate) fn mesh_stages(r: &Renderer, scene: &v2::Scene, base: &Path) -> Result<std::collections::BTreeMap<String, Stages>> {
    fn visit(nodes: &[v2::Node], names: &mut std::collections::BTreeSet<String>) {
        for node in nodes {
            match &node.kind {
                v2::NodeKind::Mesh { materials, .. } => names.extend(materials.iter().cloned()),
                v2::NodeKind::Asset { materials, .. } => names.extend(materials.values().cloned()),
                _ => {}
            }
            visit(&node.children, names);
        }
    }
    let mut names = std::collections::BTreeSet::new();
    visit(&scene.nodes, &mut names);
    let builtin=Stages {surface:true,medium:false,displacement:false};
    let mut programs=std::collections::BTreeMap::new();
    for name in &names {
        let program=scene.materials[name].program.as_str();
        if program!="std/openpbr" {programs.entry(program).or_insert(name);}
    }
    let stages=if programs.is_empty() {std::collections::BTreeMap::new()} else {
        let compiler=Compiler::discover();
        let mut abi=abi();
        abi.aov_count=scene.render.aovs.len();
        // Stage discovery shares the same source/import identity as reflection.
        // Deduplicate before launching workers so aliases cannot race cache misses.
        let mut groups=std::collections::BTreeMap::<(String,PathBuf),Vec<String>>::new();
        for (program,_) in programs {
            let (source,origin)=material_source(scene,base,program)?;
            groups.entry((source,origin.to_path_buf())).or_default().push(program.to_owned());
        }
        let programs:Vec<_>=groups.into_iter().collect();
        let probe=|((source,origin),names):&((String,PathBuf),Vec<String>)| -> Result<Vec<(String,Stages)>> {
            let identity=program_identity(&r.disk_cache,&compiler,&abi,source,origin)?;
            let stages=cached(r, input_key(b"mesh-stages-v5", &abi, &identity.identity)?.finalize(), &compiler.with_identity(&identity), &abi, source, origin, |compiler| {
                let (declarations, _) = compiler.stages(&abi, source, origin)?;
                let stages = Stages {
                    surface: declarations.contains("surface"),
                    medium: declarations.contains("medium"),
                    displacement: declarations.contains("displace"),
                };
                ensure!(stages.surface || stages.medium, "program {}: mesh program must export surface or medium; lit() and displace() supplement surface(), they do not replace it; see vork help programs --all",names[0]);
                Ok(stages)
            })?;
            Ok(names.iter().map(|name|(name.clone(),stages)).collect())
        };
        parallel_map(&programs, probe)?.into_iter().flatten().collect::<std::collections::BTreeMap<_,_>>()
    };
    Ok(names.into_iter().map(|name| {
        let program=&scene.materials[&name].program;
        let mut stage=if program=="std/openpbr" {builtin} else {stages[program]};
        if scene.media_disabled { stage.medium=false; }
        (name,stage)
    }).collect())
}

/// Whether any authored material is a program instance. Used only to avoid
/// speculatively compiling the built-in registry before install filters roots.
pub(crate) fn has_programs(scene: &crate::Scene) -> bool {
    scene.materials.values().any(|m| {
        m.program != "std/openpbr"
            || m.inputs
                .values()
                .any(|v| matches!(v, v2::Input::Attribute(_)))
    })
}

/// Compile every distinct program among `materials` (material name and its
/// program instance); None when there are none.
pub(crate) fn install<'a>(
    r: &Renderer,
    scene: &v2::Scene,
    materials: impl Iterator<Item = (&'a str, &'a Instance)>,
    base: &Path,
) -> Result<Option<Programs>> {
    let mut abi = abi();
    abi.aov_count = scene.render.aovs.len();
    let named: Vec<_> = materials.collect();
    let materials: Vec<_> = named.iter().map(|(_, p)| *p).collect();
    let mut sources = Vec::<(String, PathBuf)>::new();
    let mut ids = HashMap::new();
    for p in &materials {
        let key = instance_key(p);
        if ids.contains_key(&key) {
            continue;
        }
        let (source, origin) = if !scene.programs.contains_key(&p.name) && vork_slang::module_source(&p.name).is_none() { (p.source.clone().context("imported program source")?,base.join("imported.slang")) } else { material_source(scene, base, &p.name)? };
        let index = match sources
            .iter()
            .position(|(s, o)| *s == source && o.parent() == origin.parent())
        {
            Some(i) => i,
            None => {
                sources.push((source, origin));
                sources.len() - 1
            }
        };
        ids.insert(key, index);
    }
    if sources.is_empty() && crate::attributes::names(scene)?.is_empty() {
        return Ok(None);
    }
    let compiler = Compiler::discover();
    let compiled: Vec<Program> = parallel_map(&sources, |(source, origin)| compile(r, &compiler, &abi, source, origin))?;
    // Raw grouping above only avoids duplicate work. Program routing must use
    // compiler identity too: commenting one of two identical aliases must not
    // create another program bin or grow the scene dispatcher.
    let identities=compiled.iter().map(|p|p.install_identity.as_ref().map(|i|i.identity.as_slice()).context("material input identity")).collect::<Result<Vec<_>>>()?;
    let remap=semantic_indices(identities.into_iter());
    let mut unique=Vec::new();
    let mut unique_sources=Vec::new();
    for (index,(program,source)) in compiled.into_iter().zip(sources).enumerate() {
        if remap[index]==unique.len() {unique.push(program);unique_sources.push(source);}
    }
    for index in ids.values_mut() {*index=remap[*index];}
    let mut compiled=unique;
    let sources=unique_sources;
    for (program,(_,origin)) in compiled.iter().zip(&sources) {
        if !program.warnings.trim().is_empty() {eprintln!("{}: {}",origin.display(),program.warnings.trim());}
    }
    let mut dynamic_inputs = HashMap::<String, std::collections::BTreeSet<String>>::new();
    for track in scene.animation.iter().flat_map(|layer| layer.tracks.iter().flatten()) {
        if let [root, name, inputs, field] = track.path.as_slice() {
            if root == "materials" && inputs == "inputs" {
                if let Some(p) = instance(scene, name)? {
                    dynamic_inputs.entry(instance_key(&p)).or_default().insert(field.clone());
                }
            }
        }
    }
    // Prove effects and temporal coverage from each instance, independently of
    // opacity semantics. Fully opaque results need no coverage integration.
    let mut probed = std::collections::BTreeMap::<String, (&str, &Instance)>::new();
    for &(name, p) in &named {
        let c = compiled[ids[&instance_key(p)]].capabilities;
        let response=c.anisotropy || c.film || c.coat || c.fuzz || c.tint;
        if response || c.emission || c.hair || c.transmission || c.subsurface || c.spatial_subsurface || c.opacity {
            probed.entry(instance_key(p)).or_insert((name,p));
        }
    }
    let mut groups = std::collections::BTreeMap::<usize, Vec<(String, &str, &Instance)>>::new();
    for (instance, (name, p)) in probed { groups.entry(ids[&instance]).or_default().push((instance, name, p)); }
    let groups: Vec<_> = groups.into_iter().collect();
    let probed = parallel_map(&groups, |(index, instances)| {
        let program = &compiled[*index];
        let probe = program.effect_probe()?;
        instances.iter().map(|(instance, name, p)| {
            let dynamic = dynamic_inputs.get(instance).cloned().unwrap_or_default();
            let words = program.inputs.effect_words(&p.inputs, &dynamic)
                .with_context(|| format!("materials.{name}.inputs: program {}: closure effects", p.name))?;
            let live_texels = p.images.iter().any(|i| i.source.changes_between_frames());
            let varies = !dynamic.is_empty() || probe.coverage_varies(&words, live_texels);
            Ok((instance.clone(), probe.capabilities(&words), varies))
        }).collect::<Result<Vec<_>>>()
    })?;
    let mut bound = HashMap::new();
    let mut coverage_varies = HashMap::new();
    for (instance, capabilities, varies) in probed.into_iter().flatten() {
        bound.insert(instance.clone(), capabilities);
        coverage_varies.insert(instance, varies);
    }
    let physical_hair = bound.values().any(|c| c.physical_hair);
    validate_bound_geometry(scene, &ids, &mut compiled)?;
    Ok(Some(Programs {
        identity: format!("{:?}{}{}",crate::attributes::names(scene)?,physical_hair,compiled.iter().map(|p| p.wgsl.as_str()).collect::<String>()),
        native: if compiled.is_empty() {
            None
        } else {
            Some(std::sync::Arc::new({
                let mut all=native(&abi, &compiled, physical_hair, false)?;
                all.receivers=Some(Box::new(native(&abi,&compiled,physical_hair,true)?));
                all.camera=compiled.iter().zip(&sources).map(|(p,(_,origin))| {
                    let mut one=native(&abi,std::slice::from_ref(p),physical_hair,false)?;
                    one.name=origin.display().to_string();
                    // Primary evaluation is dispatched by bin, so program ids never
                    // enter its source or cache key. Secondary hits use the full table.
                    one.code=one.code.replace("record.tag.x", "1u")
                        .replace("material_records[material_record(v.object)].tag.x", "1u");
                    for name in ["program_dispatch","hair_dispatch","lobe_dispatch","lit_dispatch","displacement_dispatch","aov_dispatch"] {
                        one.code=one.code.replace(name,&format!("primary_{name}"));
                        for (module,text) in &mut one.modules {
                            if module==name {*module=format!("primary_{name}");}
                            *text=text.replace(name,&format!("primary_{name}"));
                        }
                    }
                    Ok(one)
                }).collect::<Result<_>>()?;
                all
            }))
        },
        compiled,
        bound,
        coverage_varies,
        physical_hair,
        aovs: scene.render.aovs.iter().enumerate().map(|(i,a)|(a.name.clone(),i as u32)).collect(),
        instances: ids,
    }))
}

/// One cached compile. The key includes compiler tokens and resolved imports;
/// proof subprocesses verify their captured imports before publication.
pub(crate) fn compile(
    r: &Renderer,
    compiler: &Compiler,
    abi: &Abi,
    source: &str,
    origin: &Path,
) -> Result<Program> {
    let authored_origin=origin;
    let identity=program_identity(&r.disk_cache,compiler,abi,source,&origin)?;
    let key = input_key(b"program-v4", abi, &identity.identity)?.finalize();
    let mut program = cached(
        r,
        key,
        &compiler.with_identity(&identity),
        abi, source, &origin,
        |compiler| {
            let tag = &key.to_hex()[..12];
            let program = compiler
                .compile_surface(abi, source, &origin, tag)
                .with_context(|| format!("compile {}", origin.display()))?;
            Ok(program)
        },
    )?;
    program.set_native_source(source, origin)?;
    program.install_identity=Some(identity);
    for stage in &program.dropped_stages {
        crate::warnings::dropped("hair_stage", serde_json::json!({"program":authored_origin,"stage":stage}),
            format!("{stage} stage on Hair; retaining the physical hair surface"));
    }
    Ok(program)
}

fn validate_bound_geometry(
    scene: &v2::Scene,
    ids: &HashMap<String, usize>,
    compiled: &mut [Program],
) -> Result<()> {
    fn visit(
        scene: &v2::Scene,
        nodes: &[v2::Node],
        ids: &HashMap<String, usize>,
        compiled: &mut [Program],
    ) -> Result<()> {
        for node in nodes {
            let (materials, curves): (Vec<&String>, bool) = match &node.kind {
                v2::NodeKind::Mesh {
                    geometry,
                    materials,
                    ..
                } => (
                    materials.iter().collect(),
                    matches!(scene.geometry[geometry].topology, v2::Topology::Curves),
                ),
                v2::NodeKind::Asset { materials, .. } => (materials.values().collect(), false),
                _ => (Vec::new(), false),
            };
            for name in materials {
                if scene.materials[name].program=="std/openpbr" {
                    for (input,value) in &scene.materials[name].inputs {
                        if let v2::Input::Attribute(binding)=value {
                            if let v2::NodeKind::Mesh {geometry,..}=&node.kind {
                                let buffer=crate::attributes::resolve(scene,node,&binding.attribute).with_context(||format!("material {name}"))?;
                                let width=match vork_scene::OpenPbrParam::find(input).context("OpenPBR input")?.kind {vork_scene::OpenPbrKind::Float=>1,vork_scene::OpenPbrKind::Color3=>3,_=>anyhow::bail!("attribute requires float or colour destination")};
                                crate::attributes::validate(binding,width,crate::attributes::arity(scene.buffers[buffer].element_type)?).with_context(||format!("material {name}: input {input}: geometry {geometry}"))?;
                            }
                        }
                    }
                }
                if let Some(instance) = instance(scene, name)? {
                    if let Some(&i) = ids.get(&instance_key(&instance)) {
                        for image in &instance.images {
                            if compiled[i].capabilities.displacement && compiled[i].displacement_images.contains(&image.input) && image.source.changes_between_frames() {
                                crate::warnings::dropped("displacement_image", serde_json::json!({"material":name,"program":scene.materials[name].program,"image":image.input,"inputs":scene.materials[name].inputs}), format!("warning: material {name}: ignored displacement sampling a changing/render image before geometry evaluation"));
                                compiled[i].capabilities.displacement=false;
                            }
                        }

                        for field in &compiled[i].inputs.fields {
                            if let vork_slang::Type::Attribute(width)=field.ty {
                                let material=&scene.materials[name];
                                let Some(v2::Input::Attribute(binding))=material.inputs.get(&field.name) else {anyhow::bail!("material {name}: Attribute input {} requires an attribute binding",field.name)};
                                if let v2::NodeKind::Mesh {geometry,..}=&node.kind {
                                    let buffer=crate::attributes::resolve(scene,node,&binding.attribute).with_context(||format!("material {name}"))?;
                                    let arity=crate::attributes::arity(scene.buffers[buffer].element_type)?;
                                    crate::attributes::validate(binding,width,arity).with_context(||format!("material {name}: input {}: geometry {geometry}",field.name))?;
                                }
                            }
                        }
                        ensure!(
                            !compiled[i].capabilities.hair || curves,
                            "material {name}: Hair surface requires curves topology"
                        );
                    }
                }
            }
            visit(scene, &node.children, ids, compiled)?;
        }
        Ok(())
    }
    visit(scene, &scene.nodes, ids, compiled)
}

/// Map `f` over `items` on up to one thread per core, keeping their order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> Result<R> + Sync) -> Result<Vec<R>> {
    let warning_collector = crate::warnings::Collector::current();
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers = items.len().min(std::thread::available_parallelism().map_or(1, usize::from));
        let jobs: Vec<_> = (0..workers).map(|_| scope.spawn(|| {
            let _warning_scope = warning_collector.as_ref().map(|c| c.enter());
            let mut results = Vec::new();
            loop {
                let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(item) = items.get(index) else { break };
                results.push((index, f(item)?));
            }
            Ok::<_, anyhow::Error>(results)
        })).collect();
        let mut results = Vec::new();
        for job in jobs { results.extend(job.join().expect("program worker panicked")?); }
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, result)| result).collect())
    })
}

fn semantic_indices<'a>(identities:impl Iterator<Item=&'a [u8]>)->Vec<usize> {
    let mut unique=std::collections::BTreeMap::new();
    identities.map(|identity| {
        let next=unique.len();
        *unique.entry(identity).or_insert(next)
    }).collect()
}

/// The cache key prefix of a proof about one compiler-input snapshot.
fn input_key(tag: &[u8], abi: &Abi, identity: &[u8]) -> Result<blake3::Hasher> {
    let mut key = blake3::Hasher::new();
    let abi_identity=abi.cache_identity()?;
    for part in [tag,abi_identity.as_bytes(),identity] {
        key.update(&(part.len() as u64).to_le_bytes());key.update(part);
    }
    Ok(key)
}

fn program_identity(store: &disk_cache::Store, compiler: &Compiler, abi: &Abi, source: &str, origin: &Path) -> Result<vork_slang::InputIdentity> {
    bound_program_identity(store,compiler,abi,source,origin,"")
}

pub(crate) fn bound_program_identity(
    store: &disk_cache::Store, compiler: &Compiler, abi: &Abi,
    source: &str, origin: &Path, bindings: &str,
) -> Result<vork_slang::InputIdentity> {
    let mut raw=input_key(b"program-input-receipt-v2",abi,source.as_bytes())?;
    raw.update(&(bindings.len() as u64).to_le_bytes());raw.update(bindings.as_bytes());
    for part in [compiler.input_policy(),std::env::current_dir()?.join(origin).to_string_lossy().into_owned()] {
        raw.update(&(part.len() as u64).to_le_bytes());raw.update(part.as_bytes());
    }
    let path=store.path("slang-input",1,raw.finalize());
    if let Some(bytes)=disk_cache::read(&path)
        && let Ok(identity)=serde_json::from_slice::<vork_slang::InputIdentity>(&bytes)
        && identity.valid() { return Ok(identity); }
    let prepared=compiler.prepare_bound_program(abi,source,origin,bindings)?;
    let identity=prepared.receipt();
    let _=disk_cache::write(&path,&serde_json::to_vec(&identity)?);
    Ok(identity)
}

/// Exact-input receipts and normalized identities use the same native artifact
/// store. A miss retains the captured filesystem for subsequent code generation.
pub(crate) fn native_input_identity(
    cache: &dyn Fn(blake3::Hash)->PathBuf, compiler: &Compiler,
    modules: &[(&str,&str)], main: &str, entries: &[(&str,&str)], args: &[&str],
    identities: &std::collections::BTreeMap<String,Vec<u8>>,
) -> Result<(vork_slang::InputIdentity,Option<vork_slang::PreparedModules>)> {
    let path=native_input_receipt_path(cache,compiler,modules,main,entries,args,identities)?;
    if let Some(identity)=read_native_input_receipt(&path) {return Ok((identity,None));}
    let prepared=compiler.prepare_identified_modules(modules,main,entries,args,identities)?;
    let identity=prepared.receipt();
    let _=disk_cache::write(&path,&serde_json::to_vec(&identity)?);
    Ok((identity,Some(prepared)))
}

/// Scheduling probes must never invoke Slang: an unknown receipt is a miss.
/// Only the build owns preparation, retaining its snapshot through codegen.
pub(crate) fn cached_native_input_identity(
    cache: &dyn Fn(blake3::Hash)->PathBuf, compiler: &Compiler,
    modules: &[(&str,&str)], main: &str, entries: &[(&str,&str)], args: &[&str],
    identities: &std::collections::BTreeMap<String,Vec<u8>>,
) -> Result<Option<vork_slang::InputIdentity>> {
    Ok(read_native_input_receipt(&native_input_receipt_path(cache,compiler,modules,main,entries,args,identities)?))
}

fn read_native_input_receipt(path: &Path) -> Option<vork_slang::InputIdentity> {
    let identity: vork_slang::InputIdentity=serde_json::from_slice(&disk_cache::read(path)?).ok()?;
    identity.valid().then_some(identity)
}

fn native_input_receipt_path(
    cache: &dyn Fn(blake3::Hash)->PathBuf, compiler: &Compiler,
    modules: &[(&str,&str)], main: &str, entries: &[(&str,&str)], args: &[&str],
    identities: &std::collections::BTreeMap<String,Vec<u8>>,
) -> Result<PathBuf> {
    let mut raw=blake3::Hasher::new();
    for part in std::iter::once("native-input-receipt-v1").chain(std::iter::once(main))
        .chain(entries.iter().flat_map(|(entry,stage)|[*entry,*stage])).chain(args.iter().copied())
        .chain(modules.iter().flat_map(|(name,source)|[*name,*source])) {
        raw.update(&(part.len() as u64).to_le_bytes());raw.update(part.as_bytes());
    }
    raw.update(compiler.input_policy().as_bytes());
    for (name,identity) in identities {raw.update(&(name.len() as u64).to_le_bytes());raw.update(name.as_bytes());raw.update(&(identity.len() as u64).to_le_bytes());raw.update(identity);}
    // Exact resolver receipts contain host paths. Keep them in the same store
    // but outside the four portable shader entry kinds shipped with releases.
    let artifact_path=cache(raw.finalize());
    let name=artifact_path.file_name().and_then(|n|n.to_str()).and_then(|n|n.split_once('.')).context("native cache file name")?.0;
    let path=artifact_path.with_file_name(format!("{name}.slang-input-v1.entry"));
    Ok(path)
}

#[cfg(test)]
fn program_key(tag: &[u8], abi: &Abi, source: &str, origin: &Path) -> Result<blake3::Hasher> {
    let prepared=Compiler::discover().prepare_program(abi,source,&origin.to_path_buf())?;
    input_key(tag,abi,prepared.identity())
}

pub(crate) fn cached<T: serde::Serialize + serde::de::DeserializeOwned>(
    r: &Renderer,
    key: blake3::Hash,
    compiler: &Compiler,
    abi: &Abi, source: &str, origin: &Path,
    build: impl FnOnce(&Compiler) -> Result<T>,
) -> Result<T> {
    cached_proof(&r.disk_cache,key,compiler,abi,source,origin,|compiler| {
        let start=std::time::Instant::now();
        let result=build(compiler);
        crate::timing::creation(&r.device,&format!("program {key}"),"Slang",start.elapsed().as_secs_f64()*1000.,0.);
        result
    })
}

/// All proof stages share exact compiler receipts. Stage ABI, generated bindings
/// and options belong in the caller key; the compiler policy belongs here.
fn cached_proof<T: serde::Serialize + serde::de::DeserializeOwned>(
    store: &disk_cache::Store, key: blake3::Hash, compiler: &Compiler,
    abi: &Abi, source: &str, origin: &Path,
    build: impl FnOnce(&Compiler) -> Result<T>,
) -> Result<T> {
    let identity=match compiler.input_identity() {
        Some(identity)=>identity.clone(),
        None=>program_identity(store,compiler,abi,source,origin)?,
    };
    let key=input_key(key.as_bytes(),abi,&identity.identity)?.finalize();
    let path=store.path("slang-program",2,key);
    if identity.valid()
        && let Some(bytes)=disk_cache::read(&path)
        && let Ok(program)=serde_json::from_slice::<T>(&bytes) {return Ok(program);}
    let (program,receipt)=compiler.with_identity(&identity).capture_inputs(build)?;
    receipt.verify_proof(&identity)?;
    if identity.valid() && receipt.valid() {
        let _=disk_cache::write(&path,&serde_json::to_vec(&program)?);
    }
    Ok(program)
}

/// Complete generated shaders have no authored resolver inputs. Their caller
/// keys include the whole validated source, binding map and compiler options.
pub(crate) fn cached_generated<T: serde::Serialize + serde::de::DeserializeOwned>(
    r: &Renderer, key: blake3::Hash, build: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let path=r.disk_cache.path("slang-program",2,key);
    if let Some(bytes)=disk_cache::read(&path)
        && let Ok(program)=serde_json::from_slice::<T>(&bytes) {return Ok(program);}
    let start=std::time::Instant::now();
    let program=build();
    crate::timing::creation(&r.device,&format!("program {key}"),"Slang",start.elapsed().as_secs_f64()*1000.,0.);
    let program=program?;
    let _=disk_cache::write(&path,&serde_json::to_vec(&program)?);
    Ok(program)
}

/// A scene's programs linked into the Slang colour passes (docs/design/shaders.md,
/// Programs). Each program is its own module, so its identifiers need no tags;
/// `program_dispatch` is the only module that imports them, and the variant module
/// applies its result through `struct Programs : IProgram`. Context and closure
/// cross as packed vec4s: one module cannot see both `vork` and the renderer's
/// Closure, and the packing inlines away.
#[derive(Clone)]
pub(crate) struct Native {
    /// Independent primary evaluators, indexed by installed program id minus one.
    pub(crate) camera: Vec<Native>,
    /// Volume receiver depth needs opacity, including fibre extinction, not
    /// unrelated opaque material bodies. Tags and the shared Blend ABI stay global.
    receivers: Option<Box<Native>>,
    pub(crate) coverage: bool,
    /// Modules a variant compile adds: native `vork` and its std modules, each
    /// program, and the dispatch.
    pub(crate) modules: Vec<(String, String)>,
    /// Directories program imports resolve from.
    pub(crate) include: Vec<PathBuf>,
    pub(crate) inputs: Vec<vork_slang::InputIdentity>,
    pub(crate) identities: std::collections::BTreeMap<String,Vec<u8>>,
    /// The variant module's imports and `struct Programs`.
    pub(crate) code: String,
    /// A primary evaluator's program source, for diagnostics.
    pub(crate) name: String,
}
impl Native {
    pub(crate) fn for_coverage(&self)->&Self {self.receivers.as_deref().unwrap_or(self)}
}

/// Path kernels keep each material dispatch out of line in Slang (docs/design/implementation.md).
const PATH_NOINLINE: &str = "#ifdef VORK_PATH_SLANG_OUTLINE\n[noinline]\n#endif\n";
const CLAMPS: &str = "o.closure.geometry_opacity=clamp(o.closure.geometry_opacity,0.,1.);o.closure.transmission_weight=clamp(o.closure.transmission_weight,0.,1.);\n";
fn native(abi: &Abi, programs: &[Program], physical_hair: bool, coverage_only: bool) -> Result<Native> {
    let retained=|p:&Program|!coverage_only || p.capabilities.opacity || p.capabilities.hair;
    let scattering = if physical_hair { "h[2].w!=0." } else { "false" };
    let (c, k) = (abi.context_vec4s(), abi.closure_vec4s());
    let aovs = abi.aov_count.max(1);
    let aov_count = abi.aov_count;
    let hair_k = vork_slang::HAIR_VEC4S;
    let mut modules = abi.native_modules();
    // Standalone displacement and medium coverage compiles share this exact
    // Surface layout too, including compiles outside SlangForward.
    let constants = crate::native::SOURCES.iter().find(|(name,_)| *name=="vork_constants").unwrap().1;
    modules.push(("vork_constants".into(), format!("#ifndef VORK_AOV_COUNT\n#define VORK_AOV_COUNT {aov_count}\n#endif\n{constants}")));
    if programs.iter().any(|p| p.capabilities.hair) { modules.push(("hair_lighting".into(), include_str!("../shaders/programs/hair_lighting.slang").into())); }
    let mut include = Vec::new();
    let mut inputs:Vec<vork_slang::InputIdentity>=Vec::new();
    let mut identities=std::collections::BTreeMap::new();
    let mut imports = String::new();
    let mut cases = String::new();
    let mut aov_cases=String::new();
    let mut hair_cases = String::new();
    let mut lobe_cases = String::new();
    let mut lit_cases = String::new();
    let mut displacement_cases = String::new();
    for (i, p) in programs.iter().enumerate() {
        if !retained(p) {continue;}
        if let Some(identity)=&p.install_identity {
            for previous in &inputs { identity.verify(previous)?; }
            inputs.push(identity.clone());
        }
        let tag = p
            .surface
            .strip_prefix("vork_surface_")
            .and_then(|s| s.split("_0_").next())
            .context("program surface name")?;
        // The native entry's compiler snapshot owns resolved import identity.
        modules.push((format!("program_{tag}"), p.slang.clone()));
        if let Some(input)=&p.install_identity {
            if let Some(previous)=identities.insert(format!("program_{tag}"),input.identity.clone()) {
                ensure!(previous==input.identity,"material module identifier collision");
            }
        }
        include.extend(p.include.iter().cloned());
        writeln!(imports, "import program_{tag};")?;
        if p.capabilities.hair { writeln!(hair_cases, "    case {}u: vork_surface_{tag}(a,base,r); return true;", i+1)?; } else if p.capabilities.lobes {
        writeln!(lobe_cases, "    case {}u: vork_surface_{tag}(a,base,lobe,u,r,lobes); return true;", i + 1)?;
        } else {
        writeln!(cases, "    case {}u: vork_surface_{tag}(a,base,r); return true;", i + 1)?;
        }
        if p.aov.is_some() {writeln!(aov_cases,"case {}u: vork_aov_{tag}(a,base,r); return;",i+1)?;}
        if p.capabilities.displacement {writeln!(displacement_cases, "case {}u: return vork_displace_{tag}(position,normal,time,base);",i+1)?;}
        if p.capabilities.lit {writeln!(lit_cases, "    case {}u: vork_lit_{tag}(a,base,light,r); return true;", i + 1)?;}
    }
    modules.push((
        "program_dispatch".into(),
        format!(
            "// Generated by programs.rs: the scene's material programs by record tag.\n\
             {imports}\
             {PATH_NOINLINE}public bool program_dispatch(uint program, float4 a[{c}], uint base, out float4 r[{k}]) {{\n    \
             for (uint i=0u;i<{k}u;i++) {{r[i]=float4(0.);}}\n    \
             switch (program) {{\n{cases}    default: return false;\n    }}\n}}\n\
             {PATH_NOINLINE}public void aov_dispatch(uint program,float4 a[{c}],uint base,out float4 r[{aovs}]) {{r={{}};switch(program){{{aov_cases} default:return;}}}}\n"
        ),
    ));
    if !displacement_cases.is_empty() {
        modules.push(("displacement_dispatch".into(), format!("{imports}\npublic float3 displacement_dispatch(uint program,float3 position,float3 normal,float time,uint base) {{switch(program) {{{displacement_cases} default:return float3(0.);}}}}")));
    }
    let lobes = !lobe_cases.is_empty();
    if lobes {
        modules.push(("lobe_dispatch".into(), format!("{imports}\n// Blend programs: one lobe's closure, (weight, lobe count).\n{PATH_NOINLINE}public bool lobe_dispatch(uint program,float4 a[{c}],uint base,uint lobe,float u,out float4 r[{k}],out float2 lobes) {{for(uint i=0u;i<{k}u;i++) {{r[i]=float4(0.);}}lobes=float2(1.,1.);switch(program) {{\n{lobe_cases}default:return false;}}}}")));
    }
    let lit = !lit_cases.is_empty();
    if lit {
        modules.push(("lit_dispatch".into(), format!("{imports}\n// Lit stages (Shader to RGB): the closure shaded in place of the surface's.\n{PATH_NOINLINE}public bool lit_dispatch(uint program,float4 a[{c}],uint base,float4 light[2],out float4 r[{k}]) {{for(uint i=0u;i<{k}u;i++) {{r[i]=float4(0.);}}switch(program) {{\n{lit_cases}default:return false;}}}}")));
    }
    let lit_reflects = programs.iter().any(|p| retained(p) && p.capabilities.lit && p.capabilities.lit_reflects);
    modules.push(("hair_dispatch".into(), format!("{imports}\n{PATH_NOINLINE}public bool hair_dispatch(uint program,float4 a[{c}],uint base,out float4 r[{hair_k}]) {{for(uint i=0u;i<{hair_k}u;i++) r[i]=float4(0.);switch(program) {{{hair_cases}default:return false;}}}}")));
    include.sort();
    include.dedup();
    let pack = abi.pack_context_slang(|name, element| {
        let value = match (name, element) {
            ("POSITION", _) => "v.world",
            ("NORMAL", _) => "s.closure.geometry_normal",
            ("true_normal", _) => "true_normal",
            ("incoming", _) => "k.incoming",
            ("TANGENT", _) => "v.TANGENT.xyz*select(s.front_facing,1.,-1.)",
            ("position_dx", _) => "k.position_dx",
            ("position_dy", _) => "k.position_dy",
            ("COLOR_0", _) => "v.COLOR_0",
            ("TEXCOORD_0", _) => "v.TEXCOORD_0",
            ("TEXCOORD_1", _) => "varying_uv1(v)",
            ("duv_dx", 0) => "k.uv_dx",
            ("duv_dy", 0) => "k.uv_dy",
            ("duv_dx", 1) => "k.uv1_dx",
            ("duv_dy", 1) => "k.uv1_dy",
            ("time", _) => "frame.scene.x",
            ("wavelength", _) => "vork_path_wavelength",
            // The ID channel's object id, not the internal object index.
            ("object_id", _) => "uint(objects[v.object].opacity.y)",
            ("object_index", _) => "v.object",
            ("instance_index", _) => "objects[v.object].padding.y",
            ("primitive_id", _) => "material_words[objects[v.object].reserved+14u+v.primitive]",
            ("front_facing", _) => "s.front_facing",
            ("tangent_sign", _) => "select(v.TANGENT.w<0.,-1.,1.)",
            ("sample_spread", _) => "k.material_filter",
            ("has_tangent", _) => "abs(v.TANGENT.w)<2. && dot(v.TANGENT.xyz,v.TANGENT.xyz)>=1e-12",
            ("rest_position", _) => "v.rest",
            ("rest_position_dx", _) => "k.rest_dx",
            ("rest_position_dy", _) => "k.rest_dy",
            ("unperturbed_normal", _) => "s.geometric",
            ("unperturbed_normal_dx", _) => "k.normal_dx",
            ("unperturbed_normal_dy", _) => "k.normal_dy",
            ("incoming_dx", _) => "k.incoming_dx",
            ("incoming_dy", _) => "k.incoming_dy",
            _ => return None,
        };
        Some(value.to_owned())
    });
    let coverage = programs.iter().any(|p| p.capabilities.opacity);
    // Blend programs unpack the requested lobe over the same defaults.
    let lobe_branch = if lobes {
        format!("    float2 lobes;\n    \
                 if (lobe_dispatch(record.tag.x,a,record.tag.w,lobe.index,lobe.u,r,lobes)) {{\n    \
                 o.closure=closure_defaults(s.closure.geometry_normal,v.TANGENT.xyz);\n{}    \
                 o.weight=lobes.x;o.lobes=uint(lobes.y);\n    \
                 return o;\n    }}\n", abi.unpack_closure_slang("o.closure") + CLAMPS)
    } else {
        String::new()
    };
    // The lit stage sees the surface stage's context: its shading normal, not the program's.
    let lit_stage = if lit {
        format!("    let true_normal=coordinates_true_normal(k,s.geometric)*select(s.front_facing,1.,-1.);\n    \
                 let record=material_records[material_record(v.object)];\n    \
                 var base=s;base.closure.geometry_normal=s.normal;\n    \
                 float4 a[{c}]={{}};\n{}    \
                 float4 light[2]={{float4(lighting.color,lighting.alpha),float4(lighting.direct,0.)}};\n    \
                 float4 r[{k}];\n    \
                 if (!lit_dispatch(record.tag.x,a,record.tag.w,light,r)) {{return s;}}\n    \
                 var o=s;o.weight=1.;o.lobes=1u;\n    \
                 o.closure=closure_defaults(s.normal,v.TANGENT.xyz);\n{}    \
                 // Coverage stays the surface stage's; the result does not refract.\n    \
                 o.closure.geometry_opacity=s.closure.geometry_opacity;o.closure.transmission_weight=0.;\n    \
                 return o;\n",
            pack.replace("s.closure.geometry_normal", "base.closure.geometry_normal"),
            abi.unpack_closure_slang("o.closure"))
    } else {
        "    return s;\n".into()
    };
    let lit_programs = programs
        .iter()
        .enumerate()
        .filter(|(_, p)| retained(p) && p.capabilities.lit)
        .map(|(i, _)| format!("case {}u: return true;", i + 1))
        .collect::<String>();
    let code = format!(
        "import material;\nimport frame;\nimport stream;\nimport closure;\nimport surface;\nimport program_dispatch;\nimport hair_dispatch;\n{}{}\
         struct Programs : IProgram {{\n\
         static const bool surface = true;\n\
         static const bool coverage = {coverage};\n\
         static const bool lobes = {lobes};\n\
         static const bool lit = {lit};\n\
         static const bool lit_reflects = {lit_reflects};\n\
         static bool lit_material(Varying v) {{switch (material_records[material_record(v.object)].tag.x) {{{lit_programs} default: return false;}}}}\n\
         static Surface apply_lit(Varying v, Surface s, Coordinates k, SurfaceLighting lighting) {{\n{lit_stage}}}\n\
         static Surface apply(Varying v, Surface s, Coordinates k, Lobe lobe) {{\n    \
         let true_normal=coordinates_true_normal(k,s.geometric)*select(s.front_facing,1.,-1.);\n    \
         let record=material_records[material_record(v.object)];\n    \
         if (record.tag.x==0u) {{return s;}}\n    \
         float4 a[{c}]={{}};\n{pack}    \
         var o=s;\n    \
         float4 aovs[{aovs}];if ({aov_count}u!=0u) aov_dispatch(record.tag.x,a,record.tag.w,aovs);\n    \
         o.aovs=aovs;\n    \
         float4 h[{hair_k}];\n    \
         if (hair_dispatch(record.tag.x,a,record.tag.w,h)) {{o.hair0=h[0];o.hair1=h[1];o.is_hair={scattering};o.strand=v.TANGENT.xyz;o.closure.specular_weight=0.;o.closure.coat_weight=0.;o.closure.fuzz_weight=0.;o.closure.base_color=h[2].xyz;o.closure.base_weight=select(o.is_hair,0.,1.);o.closure.base_metalness=0.;o.closure.transmission_weight=0.;return o;}}\n    \
         float4 r[{k}];\n{lobe_branch}    \
         if (!program_dispatch(record.tag.x,a,record.tag.w,r)) {{return o;}}\n    \
         // OpenPBR defaults in the unperturbed frame, then the program's fields.\n    \
         o.closure=closure_defaults(s.closure.geometry_normal,v.TANGENT.xyz);\n{}    \
         return o;\n}}\n}};\n",
        if lobes { "import lobe_dispatch;\n" } else { "" },
        if lit { "import lit_dispatch;\n" } else { "" },
        abi.unpack_closure_slang("o.closure") + CLAMPS
    );
    Ok(Native { camera:Vec::new(), receivers:None, coverage, modules, include, inputs, identities, code, name:String::new() })
}

#[cfg(test)]
mod help_tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn volume_receivers_retain_coverage_ids_hair_and_shader_capacity() -> Result<()> {
        let compiler=Compiler::discover();let abi=abi();
        let bodies=[
            "Blend surface(ShadingContext ctx,Inputs i) {Closure c={};Blend s=add(c,c);s=add(s,s);return add(s,s);}",
            "Closure surface(ShadingContext ctx,Inputs i) {Closure c={};c.geometry_opacity=i.value;return c;}",
            "Hair surface(ShadingContext ctx,Inputs i) {Hair h={};return h;}",
            "Closure surface(ShadingContext ctx,Inputs i) {Closure c={};c.base_color=float3(.314159);return c;}",
        ];
        let programs=bodies.iter().enumerate().map(|(i,body)|compiler.compile_surface(&abi,
            &format!("import vork;struct Inputs {{float value;}};{body}"),Path::new("receiver-program.slang"),&format!("receiver{i}")))
            .collect::<Result<Vec<_>>>()?;
        assert!(!programs[0].capabilities.opacity && !programs[3].capabilities.opacity);
        assert!(programs[1].capabilities.opacity && programs[2].capabilities.hair);
        assert_eq!(programs[0].capabilities.blend_lobes,Some(8));
        let receivers=native(&abi,&programs,true,true)?;
        for (i,p) in programs.iter().enumerate() {
            assert_eq!(receivers.modules.iter().any(|(_,s)|s.starts_with(&p.slang)),i==1 || i==2);
        }
        let dispatch=&receivers.modules.iter().find(|(name,_)|name=="program_dispatch").unwrap().1;
        assert!(dispatch.contains("case 2u:"));
        assert!(!dispatch.contains("case 1u:") && !dispatch.contains("case 4u:"));
        let hair=&receivers.modules.iter().find(|(name,_)|name=="hair_dispatch").unwrap().1;
        assert!(hair.contains("case 3u:"));
        for (name,text) in abi.native_modules() {
            assert_eq!(receivers.modules.iter().find(|(n,_)|*n==name).unwrap().1,text);
        }
        Ok(())
    }
    /// Programs may set the coat's shading normal (the Blender bridge writes
    /// Principled "Coat Normal" there). Unset or set to the unperturbed normal,
    /// the coat shades the same; tilted away from the half vector, its highlight
    /// falls. Before the ABI exposed it, install failed with
    /// "'geometry_coat_normal' is not a member of 'Closure'".
    #[test]
    #[cfg(vork_private_tests)]
    fn programs_shade_their_coat_normal() -> Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        let render=|integrator:&str,coat_normal:&str|->Result<[f32;3]> {
            let mut scene:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"), include_str!("../tests/presets/analytic-7fefd52a.json"))?;
            scene["animation"]=serde_json::json!([]);
            scene["render"]=serde_json::json!({"resolution":[8,8],"settings":{"integrator":integrator,"denoiser":null,"samples":4}});
            // The orthographic camera sees only the unit box's front face (+Z);
            // the sun shines along the view, so the half vector is the face normal.
            let mut camera=scene["nodes"][0].clone();camera["material"]["inputs"]["ortho_scale"]=serde_json::json!(0.5);
            scene["nodes"]=serde_json::json!([camera,scene["nodes"][1].clone(),
                {"name":"sun","type":"light","shape":"sun","energy":3}]);
            scene["nodes"][1]["materials"]=serde_json::json!(["coat"]);
            scene["programs"]=serde_json::json!({"coat":{"source":format!("import vork; struct Inputs {{}}; Closure surface(ShadingContext ctx,Inputs i) {{Closure c={{}};c.base_weight=0.;c.specular_weight=0.;c.coat_weight=1.;c.coat_roughness=.3;c.coat_ior=1.5;{coat_normal}return c;}}")}});
            scene["materials"]=serde_json::json!({"coat":{"program":"coat"}});
            crate::path_medium_tests::mean_color(&r,scene)
        };
        for integrator in ["raster","path"] {
            let unset=render(integrator,"")?;
            let unperturbed=render(integrator,"c.geometry_coat_normal=ctx.NORMAL;")?;
            // 30 degrees off the half vector: far outside a roughness .3 GGX lobe.
            let tilted=render(integrator,"c.geometry_coat_normal=ctx.NORMAL*cos(radians(30.))+float3(1.,0.,0.)*sin(radians(30.));")?;
            eprintln!("{integrator}: unset {unset:?} unperturbed {unperturbed:?} tilted {tilted:?}");
            for channel in 0..3 {
                assert!(unset[channel]>0.05,"{integrator}: no coat highlight {unset:?}");
                assert!((unset[channel]-unperturbed[channel]).abs()<=1e-3*unset[channel],"{integrator}: unset {unset:?} vs unperturbed {unperturbed:?}");
                assert!(tilted[channel]<0.5*unset[channel],"{integrator}: tilted {tilted:?} vs unset {unset:?}");
            }
        }
        Ok(())
    }
    #[test]
    fn camera_programs_deduplicate_and_keep_independent_content() -> Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        // Keep the compile-count assertion independent of an earlier test process's disk cache.
        let source=format!("{}\n// independence test {}", "import vork; struct Inputs {float value=0.25;}; Closure surface(ShadingContext ctx,Inputs i) {Closure c;c.base_color=float3(i.value);return c;}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
        let mut scene:v2::Scene=vork_scene::v2::from_test_value(serde_json::json!({"version":2,"nodes":[],
            "programs":{"a":{"source":source},"duplicate":{"source":source},"z":{"source":source.replace("i.value)","i.value*0.5)")}},
            "materials":{"a":{"program":"a"},"duplicate":{"program":"duplicate","inputs":{"value":0.75}},"z":{"program":"z"}}
        }))?;
        let install=|scene:&v2::Scene|->Result<Programs> {
            let instances=scene.materials.keys().map(|name|Ok(instance(scene,name)?.map(|p|(name.as_str(),p)))).collect::<Result<Vec<_>>>()?;
            Ok(super::install(&r,scene,instances.iter().flatten().map(|(name,p)|(*name,p)),Path::new("."))?.unwrap())
        };
        // Mesh stage discovery must also deduplicate aliases before compiling.
        scene.nodes=vork_scene::v2::from_test_value(serde_json::json!([
            {"type":"mesh","geometry":"unused","materials":["a","duplicate","z"]}
        ]))?;
        let before=crate::timing::creations(&r.device).len();
        let stages=mesh_stages(&r,&scene,Path::new("."))?;
        assert!(stages.values().all(|stage|stage.surface && !stage.medium));
        let creations=crate::timing::creations(&r.device);
        assert_eq!(creations[before..].iter().filter(|c|c.kind=="Slang").count(),2);
        scene.nodes.clear();
        let first=install(&scene)?;
        assert_eq!(first.compiled.len(),2,"identical source and input layouts share a program");
        assert_eq!(first.instances[&instance_key(&instance(&scene,"a")?.unwrap())],first.instances[&instance_key(&instance(&scene,"duplicate")?.unwrap())]);
        let unchanged=first.native.as_ref().unwrap().camera[1].clone();
        scene.programs.get_mut("a").unwrap().source=Some(source.replace("i.value)","i.value*0.8)"));
        let edited=install(&scene)?;
        let index=edited.instances[&instance_key(&instance(&scene,"z")?.unwrap())];
        let after=&edited.native.as_ref().unwrap().camera[index];
        assert_eq!(unchanged.code,after.code);
        assert_eq!(unchanged.modules,after.modules,"editing another program, even renumbering ids, keeps native artifact content");
        Ok(())
    }

    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn source_support_comes_only_from_surface_emission() -> Result<()> {
        let compiler=Compiler::discover();let abi=abi();
        for (body,lit,expected) in [
            ("return unlit(float3(0.));", "", false),
            ("return unlit(float3(1.));", "", true),
            ("return unlit(float3(10.),0.);", "", false),
            ("return unlit(float3(0.));", "Closure lit(ShadingContext c,Inputs i,Lighting l) {return unlit(float3(10.));}", false),
            ("return unlit(float3(i.strength));", "", true),
        ] {
            let source=format!("struct Inputs {{float strength=0.;}}; Closure surface(ShadingContext c,Inputs i) {{{body}}} {lit}");
            let program=compiler.compile_surface(&abi,&source,Path::new("source-support.slang"),"support")?;
            assert_eq!(program.capabilities.emission,expected,"{source}");
            if body.contains("i.strength") {
                let words=program.inputs.effect_words(&vork_scene::v2::from_test_value(serde_json::json!({"strength":0.}))?,&Default::default())?;
                assert!(!program.effect_probe()?.capabilities(&words).emission);
            }
        }
        for body in ["return blend(unlit(float3(0.)),unlit(float3(1.)),.5);", "return add(unlit(float3(0.)),unlit(float3(c.time)));"] {
            let source=format!("struct Inputs {{}}; Blend surface(ShadingContext c,Inputs i) {{{body}}}");
            let program=compiler.compile_surface(&abi,&source,Path::new("blend-source.slang"),"blendsource")?;
            assert!(program.capabilities.emission);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn source_emission_retains_only_real_time_dependencies() -> Result<()> {
        let compiler=Compiler::discover();let abi=abi();
        for (expression,timed) in [("float3(1.)",false),("float3(1.+ctx.time)",true)] {
            let source=format!("import vork; struct Inputs {{}}; Closure surface(ShadingContext ctx,Inputs inputs) {{return unlit({expression});}}");
            let program=compiler.compile_surface(&abi,&source,Path::new("emission-domain.slang"),"sourceemission")?;
            assert!(program.capabilities.emission);
            assert_eq!(program.capabilities.dependencies.time,timed);
            assert!(!program.capabilities.dependencies.camera);
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires pinned Slang toolchain"]
    fn documented_stages_compile_against_the_renderer_abi() -> Result<()> {
        // One minimal module per stage the programs help page lists; the page must list each stage.
        const STAGES: &[(&str, &str)] = &[
            ("surface", r#"import vork;
struct Inputs {};
Closure surface(ShadingContext ctx, Inputs inputs) { Closure c = {}; return c; }
"#),
            ("displace", r#"import vork;
struct Inputs { Attribute<float> height; float scale = 0.02; };
float3 displace(DisplacementContext ctx, Inputs inputs) {
    return ctx.NORMAL * read_attribute(inputs.height, ctx) * inputs.scale;
}
Closure surface(ShadingContext ctx, Inputs inputs) { return {}; }
"#),
            ("surface (Hair)", r#"import vork;
struct Inputs {};
Hair surface(ShadingContext ctx, Inputs inputs) { Hair h = {}; return h; }
"#),
            ("surface (Blend)", r#"import vork;
struct Inputs { float weight = .5; };
Blend surface(ShadingContext ctx, Inputs inputs) {
    Closure paint = {}; paint.base_color = float3(.6, .05, .02);
    Closure metal = {}; metal.base_metalness = 1.; metal.specular_roughness = .1;
    return blend(paint, metal, inputs.weight);
}
"#),
            ("aov", r#"import vork;
struct Inputs { Aov mask; };
Closure surface(ShadingContext ctx, Inputs inputs) { Closure c = {}; return c; }
void aov(ShadingContext ctx, Inputs inputs, inout AovOutputs output) {
    write_aov(output, inputs.mask, 1.);
}
"#),
            ("lit", r#"import vork;
struct Inputs {};
Closure surface(ShadingContext ctx, Inputs inputs) { Closure c = {}; return c; }
Closure lit(ShadingContext ctx, Inputs inputs, Lighting light) {
    return unlit(dot(light.color, float3(.2126, .7152, .0722)) < .2 ? float3(.1) : float3(.8));
}
"#),
            ("medium", r#"import vork;
struct Inputs {};
MediumCoefficients medium(MediumContext ctx, Inputs inputs) { MediumCoefficients m = {}; return m; }
"#),
            ("environment", r#"import vork;
struct Inputs {};
float3 environment(EnvironmentContext ctx, Inputs inputs) { return float3(.1); }
"#),
            ("composite", r#"import vork;
struct Inputs {};
float4 composite(CompositeContext ctx, Inputs inputs) { return compositor_sample(ctx.uv); }
"#),
            ("display", r#"import vork;
struct Inputs {};
float3 display(DisplayContext ctx, Inputs inputs) { return ctx.color / (1. + ctx.color); }
"#),
            ("compute", r#"import vork;
struct Inputs {};
void compute(ComputeContext ctx, Inputs inputs) {}
"#),
        ];
        let compiler=Compiler::discover();
        let abi=abi();
        let page=include_str!("../../../docs/help/programs.md");
        for (count,(stage,source)) in STAGES.iter().enumerate() {
            let name=stage.split(' ').next().unwrap();
            ensure!(name=="medium" || page.contains(&format!(" {name}(")),"programs help does not list stage {name}");
            let tag=format!("help{count}");
            let origin=PathBuf::from(format!("help-{stage}.slang"));
            match *stage {
                "surface" | "surface (Blend)" | "surface (Hair)" | "aov" | "lit" | "displace" => compiler.compile_surface(&abi,source,&origin,&tag).map(|_|())?,
                "medium" => compiler.compile_medium(&abi,source,&origin,&tag).map(|_|())?,
                "environment" => compiler.compile_environment(&abi,source,&origin,&tag).map(|_|())?,
                "composite" => compiler.compile_composite(&abi,source,&origin,&tag).map(|_|())?,
                "display" => compiler.compile_display(&abi,source,&origin,&tag).map(|_|())?,
                "compute" => compiler.compile_compute(source,&origin,crate::compute::CONTEXT_BINDINGS,1,[1,1,1]).map(|_|())?,
                _ => anyhow::bail!("unknown documented stage {stage}"),
            }
        }
        Ok(())
    }
}

/// Bind reflected numeric words to the same ordered track evaluator as nodes.
/// Resource handles and enums select structure and cannot be interpolated.
pub(crate) fn bind_inputs(
    plan: &mut crate::animation::Plan,
    material: &str,
    layout: &vork_slang::InputLayout,
    words: &mut [u32],
    base: usize,
    offset: f32,
) -> Result<()> {
    bind_input_words(plan, material, layout, words, base, offset, false)
}
fn bind_input_words(
    plan: &mut crate::animation::Plan, material: &str, layout: &vork_slang::InputLayout,
    words: &mut [u32], base: usize, offset: f32, discrete: bool,
) -> Result<()> {
    for (name, input) in plan.material_inputs().into_iter().filter(|(name, _)| name == material) {
        let (index, field) = layout.fields.iter().enumerate().find(|(_, f)| f.name == input)
            .with_context(|| format!("track materials.{name}.inputs.{input}: unknown program input"))?;
        ensure!((discrete || field.enumeration.is_none()) && (matches!(field.ty, vork_slang::Type::Float | vork_slang::Type::Vector(_) | vork_slang::Type::Int | vork_slang::Type::Uint) || (discrete && field.ty == vork_slang::Type::Bool)),
            "track materials.{name}.inputs.{input}: resource handles, booleans and enums are structural program inputs");
        let value: Vec<f32> = words[field.offset..field.offset + field.ty.words()].iter().map(|&w| match field.ty {
            vork_slang::Type::Int => w as i32 as f32,
            vork_slang::Type::Uint => w as f32,
            _ => f32::from_bits(w),
        }).collect();
        let enum_tokens = field.enumeration.as_ref().map(|e| e.cases.iter().map(|c| c.to_ascii_lowercase()).collect::<Vec<_>>());
        let default = if let Some(tokens) = &enum_tokens {
            crate::animation::Sample::String(tokens[words[field.offset] as usize].clone().into())
        } else if field.ty == vork_slang::Type::Bool {
            crate::animation::Sample::Bool(words[field.offset] != 0)
        } else { crate::animation::Sample::Numeric(value) };
        words[index / 32] |= 1 << (index % 32);
        plan.bind_material(&(name, input), vec![crate::animation::MaterialBinding {
            slot: base + field.offset, parameter: usize::MAX, storage: Some(field.ty),
            default, enum_tokens, offset,
        }])?;
    }
    Ok(())
}

pub(crate) fn sample_words(value: &crate::animation::Sample, binding: &crate::animation::MaterialBinding) -> Vec<u32> {
    let ty = binding.storage.unwrap();
    let value = match value {
        crate::animation::Sample::Numeric(value) => value,
        crate::animation::Sample::Bool(value) => return vec![u32::from(*value)],
        crate::animation::Sample::String(value) => return vec![binding.enum_tokens.as_ref().unwrap().iter().position(|v| v == value.as_ref()).unwrap() as u32],
    };
    value.iter().map(|&v| match ty {
        vork_slang::Type::Int => (v as i32) as u32,
        vork_slang::Type::Uint => v as u32,
        _ => v.to_bits(),
    }).collect()
}

/// Standalone word consumers (environment and compute) retain the common plan,
/// binding only their own instance. There is no alternate interpolation path.
pub(crate) struct InputAnimation {
    plan: crate::animation::Plan,
}
impl InputAnimation {
    pub(crate) fn new(scene: &v2::Scene, base: &Path, material: &str, layout: &vork_slang::InputLayout, words: &mut [u32]) -> Result<Option<Self>> {
        Self::with_discrete(scene, base, material, layout, words, false)
    }
    pub(crate) fn with_discrete(scene: &v2::Scene, base: &Path, material: &str, layout: &vork_slang::InputLayout, words: &mut [u32], discrete: bool) -> Result<Option<Self>> {
        if !crate::materials::animated_inputs(scene)?.contains_key(material) { return Ok(None); }
        let mut plan = crate::animation::Plan::compile_with_buffers(scene, base, None)?;
        bind_input_words(&mut plan, material, layout, words, 0, 0., discrete)?;
        Ok(Some(Self { plan }))
    }
    pub(crate) fn identity(&self) -> blake3::Hash { self.plan.input_identity() }
    pub(crate) fn words(&self, time: f32, words: &mut [u32]) {
        let mut samples = self.plan.samples();
        self.plan.sample_material_offset(time, 0., &mut samples);
        for (binding, value) in self.plan.material_values(&samples, 0.) {
            let value = sample_words(value, binding);
            words[binding.slot..binding.slot + value.len()].copy_from_slice(&value);
        }
    }
}

#[cfg(test)]
mod bundled_origin_tests {
    use super::*;
    #[test]
    fn program_diagnostics_keep_authored_filename_and_line() {
        let error = program_key(b"diagnostic", &abi(),
            "struct Inputs {};\nClosure surface(ShadingContext c, Inputs i) {return missing_author_symbol;}\n",
            Path::new("author-palette.slang")).err().expect("invalid program");
        let message = format!("{error:#}");
        assert!(message.contains("author-palette.slang:2:"), "{message}");
        assert!(!message.contains("material.slang"), "{message}");
    }

    #[test]
    fn display_proof_cache_tracks_shadowing_and_real_edits() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-display-proof-{}",std::process::id()));
        let _=std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub"))?;
        let store=disk_cache::Store::for_test(root.join("cache"));
        let compiler=Compiler::discover(); let abi=abi();
        let origin=root.join("main.slang");
        let source="import sub.wrapper; struct Inputs {}; float3 display(DisplayContext ctx, Inputs inputs){return float3(wrapped());}";
        let helper=root.join("helper.slang"); let sibling=root.join("sub/helper.slang");
        std::fs::write(root.join("sub/wrapper.slang"),"import helper; public float wrapped(){return value();}")?;
        std::fs::write(&helper,"public float value(){return 0.25;}")?;
        // Keep the raw source/options key identical through all dependency edits.
        let key=blake3::hash(b"display-proof-test");
        let build=|compiler:&Compiler|compiler.compile_display(&abi,source,&origin,"cachetest");
        let first=cached_proof(&store,key,&compiler,&abi,source,&origin,build)?;
        let hit:vork_slang::DisplayProgram=cached_proof(&store,key,&compiler,&abi,source,&origin,|_|anyhow::bail!("unchanged inputs recompiled"))?;
        assert_eq!(hit.wgsl,first.wgsl);
        std::fs::write(&sibling,"public float value(){return 0.75;}")?;
        let shadowed=cached_proof(&store,key,&compiler,&abi,source,&origin,build)?;
        assert_ne!(first.wgsl,shadowed.wgsl,"a new preferred import must never reuse stale WGSL");
        std::fs::write(&sibling,"public float value(){return 0.5;}")?;
        let edited=cached_proof(&store,key,&compiler,&abi,source,&origin,build)?;
        assert_ne!(shadowed.wgsl,edited.wgsl,"real edits must never reuse stale WGSL");
        std::fs::remove_file(&sibling)?;
        let restored=cached_proof(&store,key,&compiler,&abi,source,&origin,build)?;
        assert_eq!(restored.wgsl,first.wgsl);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn display_proof_cache_rejects_publication_races() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-display-race-{}",std::process::id()));
        let _=std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        let store=disk_cache::Store::for_test(root.join("cache"));
        let compiler=Compiler::discover(); let abi=abi();
        let origin=root.join("main.slang"); let sibling=root.join("helper.slang");
        let source="import helper; struct Inputs {}; float3 display(DisplayContext ctx, Inputs inputs){return float3(value());}";
        std::fs::write(&sibling,"public float value(){return 0.5;}")?;
        let build=|compiler:&Compiler|compiler.compile_display(&abi,source,&origin,"cachetest");
        let race_key=blake3::hash(b"display-publication-race");
        let raced:Result<vork_slang::DisplayProgram>=cached_proof(&store,race_key,&compiler,&abi,source,&origin,|compiler| {
            let program=build(compiler)?;
            // Deterministically edit after compiler reads but before publication.
            std::fs::write(&sibling,"public float value(){return 0.875;}")?;
            Ok(program)
        });
        assert!(raced.is_err(),"old output must not be associated with new bytes");
        let mut rebuilt=false;
        let fresh=cached_proof(&store,race_key,&compiler,&abi,source,&origin,|compiler| {rebuilt=true;build(compiler)})?;
        assert!(rebuilt,"a raced compilation must not publish a cache entry");
        assert!(fresh.wgsl.contains("0.875"));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn display_proof_cache_rejects_unidentified_imports() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-proof-extra-inputs-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("helper.slang"),"public float value(){return 0.75;}")?;
        let store=disk_cache::Store::for_test(root.join("cache"));
        let compiler=Compiler::discover(); let abi=abi(); let origin=root.join("main.slang");
        let source="struct Inputs {}; float3 display(DisplayContext ctx, Inputs i){return float3(0.25);}";
        let actual="import helper; struct Inputs {}; float3 display(DisplayContext ctx, Inputs i){return float3(value());}";
        let key=blake3::hash(b"proof-extra-inputs");
        let result=cached_proof(&store,key,&compiler,&abi,source,&origin,|compiler|
            compiler.compile_display(&abi,actual,&origin,"extra"));
        assert!(result.is_err(),"a wrapper-only dependency must not escape the proof key");
        let mut rebuilt=false;
        let fresh=cached_proof(&store,key,&compiler,&abi,source,&origin,|compiler| {
            rebuilt=true;compiler.compile_display(&abi,source,&origin,"extra")
        })?;
        assert!(rebuilt,"an incomplete identity must not publish an artifact");
        assert!(fresh.wgsl.contains("0.25"));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn embedded_proof_cache_remains_portable_across_origins() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-portable-proof-{}",std::process::id()));
        let store=disk_cache::Store::for_test(root.clone());
        let key=blake3::hash(b"portable-display-proof");
        let source="struct Inputs {}; float3 display(DisplayContext ctx, Inputs inputs){return ctx.color;}";
        let abi=abi(); let compiler=Compiler::discover();
        let first_origin=root.join("setup/embedded.slang");
        let other_origin=root.join("scene/embedded.slang");
        std::fs::create_dir_all(first_origin.parent().unwrap())?;
        std::fs::create_dir_all(other_origin.parent().unwrap())?;
        let first=cached_proof(&store,key,&compiler,&abi,source,&first_origin,|compiler|
            compiler.compile_display(&abi,source,&first_origin,"portable"))?;
        let hit:vork_slang::DisplayProgram=cached_proof(&store,key,&compiler,&abi,source,&other_origin,
            |_|anyhow::bail!("embedded proof lost portability"))?;
        assert_eq!(first.wgsl,hit.wgsl);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn all_authored_stages_use_checked_cache_boundary() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-stage-smoke-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let store=disk_cache::Store::for_test(root.join("cache"));
        let compiler=Compiler::discover();let abi=abi();
        macro_rules! stage {($name:literal,$source:expr,$method:ident)=>{{
            let source=$source;let origin=root.join(concat!($name,".slang"));
            let key=blake3::hash($name.as_bytes());
            let _:serde_json::Value=cached_proof(&store,key,&compiler,&abi,source,&origin,|compiler|
                Ok(serde_json::to_value(compiler.$method(&abi,source,&origin,"stagesmoke")?)?))?;
            let _:serde_json::Value=cached_proof(&store,key,&compiler,&abi,source,&origin,
                |_|anyhow::bail!("unchanged {} stage recompiled",$name))?;
        }}}
        stage!("surface","struct Inputs {}; Closure surface(ShadingContext c,Inputs i){Closure o={};o.base_color=float3(0.3);return o;}",compile_surface);
        stage!("camera",vork_slang::module_source("std/camera").unwrap(),compile_camera);
        stage!("environment","struct Inputs {}; float3 environment(EnvironmentContext c,Inputs i){return float3(c.direction.y);}",compile_environment);
        stage!("medium",vork_slang::module_source("std/fog").unwrap(),compile_medium);
        stage!("composite","struct Inputs {float gain=1.;}; float4 composite(CompositeContext c,Inputs i){return compositor_sample(c.uv)*i.gain;}",compile_composite);
        stage!("propagation","struct Inputs {}; PropagationStep propagation(PropagationContext c,Inputs i){PropagationStep s;s.direction=c.direction;s.distance=1.;return s;}",compile_propagation);
        let source="struct Inputs {uint amount=3;}; void compute(ComputeContext c,Inputs i){result_store(c.id.x,i.amount);}";
        let bindings="struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick; struct Offset {uint4 value;}; [[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset; [[vk::binding(2,0)]] RWStructuredBuffer<uint> result; void result_store(uint i,uint v){result[i]=v;}";
        let origin=root.join("compute.slang");let key=blake3::hash(b"compute");
        let inputs=bound_program_identity(&store,&compiler,&abi,source,&origin,bindings)?;
        let compiler=compiler.with_identity(&inputs);
        let _:vork_slang::ComputeProgram=cached_proof(&store,key,&compiler,&abi,source,&origin,|compiler|
            compiler.compile_compute(source,&origin,bindings,64,[1,1,1]))?;
        let _:vork_slang::ComputeProgram=cached_proof(&store,key,&compiler,&abi,source,&origin,
            |_|anyhow::bail!("unchanged compute stage recompiled"))?;
        assert!(compiler.prepare_bound_program(&abi,"struct Inputs {}; uint bad=vork_tick.frame;",&origin,bindings).is_err(),"generated bindings must not relax authored reserved-name checks");
        std::fs::remove_dir_all(root)?;Ok(())
    }

    #[test]
    fn semantic_program_aliases_keep_ids_when_only_comments_change() -> Result<()> {
        let compiler=Compiler::discover(); let abi=abi(); let origin=Path::new("material.slang");
        let source="struct Inputs {}; Closure surface(ShadingContext ctx,Inputs i){Closure c={};c.base_color=float3(0.25);return c;}";
        let base=compiler.prepare_program(&abi,source,origin)?;
        let comment=compiler.prepare_program(&abi,&format!("// alias description\n{source}"),origin)?;
        let edited=compiler.prepare_program(&abi,&source.replace("0.25","0.75"),origin)?;
        assert_eq!(semantic_indices([base.identity(),comment.identity(),edited.identity(),base.identity()].into_iter()),vec![0,0,1,0]);
        Ok(())
    }

    #[test]
    fn native_store_reuses_comments_and_rebuilds_imported_values() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-program-identity-store-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let store=disk_cache::Store::for_test(root.join("cache"));
        let cache=|key|store.path("slang-variant",1,key);
        let helper=root.join("helper.slang");
        std::fs::write(&helper,"public float value(){return 0.25;}")?;
        let main="import helper; RWStructuredBuffer<float> result; [numthreads(1,1,1)] void main(uint3 id:SV_DispatchThreadID){result[id.x]=value();}";
        let compiler=Compiler::discover();
        let include=root.display().to_string();
        let args=["-target","spirv","-I",include.as_str()];
        let entries=[("main","compute")];
        let modules=[("main",main)];
        assert!(cached_native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?.is_none(),"cold scheduling probe must only report a miss");
        let (original,prepared)=native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?;
        let binary=prepared.context("first input must be captured")?.compile()?;
        let artifact=cache(blake3::hash(&original.identity));
        disk_cache::write(&artifact,&binary)?;
        let (_,prepared)=native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?;
        ensure!(prepared.is_none(),"exact unchanged input should reuse its receipt");
        assert_eq!(cached_native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?.unwrap().identity,original.identity);
        std::fs::write(&helper,"// explanatory comment\npublic float value() { return 0.25; }")?;
        assert!(cached_native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?.is_none(),"changed resolver input must not be reported stored");
        let (comment,prepared)=native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?;
        ensure!(prepared.is_some(),"changed bytes require compiler revalidation");
        assert_eq!(disk_cache::read(&cache(blake3::hash(&comment.identity))),Some(binary.clone()),"comment must hit the existing artifact");
        std::fs::write(&helper,"public float value(){return 0.75;}")?;
        let (edited,prepared)=native_input_identity(&cache,&compiler,&modules,"main",&entries,&args,&Default::default())?;
        assert!(disk_cache::read(&cache(blake3::hash(&edited.identity))).is_none(),"real imported edit must miss the store");
        assert_ne!(binary,prepared.context("edited input must be captured")?.compile()?);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn setup_surface_artifacts_are_reused_from_another_scene_directory() -> Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        let scene:v2::Scene=serde_json::from_value(serde_json::json!({"version":2,"nodes":[]}))?;
        let (source,origin)=material_source(&scene,Path::new("/setup"),"std/hair")?;
        let abi=abi();
        let first=compile(&r,&Compiler::discover(),&abi,&source,&origin)?;
        let (source,origin)=material_source(&scene,Path::new("/scenes/shot"),"std/hair")?;
        let key=program_key(b"program-v4",&abi,&source,&origin)?.finalize();
        let second:Program=cached(&r,key,&Compiler::discover(),&abi,&source,&origin,
            |_|anyhow::bail!("bundled hair recompiled in another scene directory"))?;
        assert_eq!(first.slang,second.slang);
        assert_eq!(first.wgsl,second.wgsl);
        Ok(())
    }

    #[test]
    fn bundled_material_origins_are_independent_of_the_scene_directory() -> Result<()> {
        let scene:v2::Scene=serde_json::from_value(serde_json::json!({"version":2,"nodes":[]}))?;
        for name in ["std/hair", "std/fog"] {
            let a=material_source(&scene,Path::new("/setup"),name)?;
            let b=material_source(&scene,Path::new("/scenes/shot"),name)?;
            assert_eq!(a,b);
            assert_eq!(a.1,PathBuf::from(format!("{name}.slang")));
            assert_eq!(program_key(b"program-v2",&abi(),&a.0,&a.1)?.finalize(),
                program_key(b"program-v2",&abi(),&b.0,&b.1)?.finalize());
        }
        let program:v2::Program=serde_json::from_value(serde_json::json!({"source":"import local;"}))?;
        let a=program_source(Path::new("/one"),"custom",&program)?;
        let b=program_source(Path::new("/two"),"custom",&program)?;
        assert_ne!(a.1.parent(),b.1.parent(),"authored relative imports retain their scene directory");
        Ok(())
    }
}

#[cfg(all(test, vork_private_tests))]
#[path = "programs_texture_tests_private.rs"]
mod texture_install_tests;
