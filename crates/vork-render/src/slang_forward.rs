//! The Slang shader backend for built-in and programmed scenes (docs/design/shaders.md).
//! Depth roles use the precompiled depth module; the control roles (opaque and
//! transparent controls) use the precompiled all-capabilities generic
//! (the superset fallback) or a variant compiled at runtime by slangc, whose native
//! code is kept in the disk cache. Media link the same transport as resolve.
use crate::{
    disk_cache,
    forward_variant::{FeatureMask, ForwardKey, METAL_TARGET, NATIVE_OPTIMIZATION, SLANG_FLAGS, SPIRV_TARGET, Stage},
    native, programs,
    pipelines::{GeometryKey, Role, VariantShader, try_init},
};
use anyhow::Context;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

/// What a scene adds to the Slang modules: its material programs and its named
/// geometry attributes (VORK_ATTRIBUTES; then the stages carrying full varyings
/// compile at install too), and the shared mesh-medium transport.
#[derive(Clone, Default)]
pub(crate) struct SlangScene {
    pub(crate) program_trace: bool,
    pub(crate) propagation: Option<crate::propagation::Native>,
    pub(crate) camera: Option<Arc<crate::camera_program::Native>>,
    pub(crate) environment: Option<crate::environment_program::Primary>,
    pub(crate) programs: Option<Arc<programs::Native>>,
    pub(crate) attributes: u32,
    pub(crate) textures: u32,
    pub(crate) samplers: u32,
    pub(crate) aov_count: usize,
    pub(crate) ray_refraction: bool,
    pub(crate) connection_transmission: bool,
    /// Installed coverage proof, shared by table, imported and program materials.
    pub(crate) surface_coverage: bool,
    pub(crate) attribute_coverage: bool,
    pub(crate) shadow_capacity: u32,
    pub(crate) no_shadow_containment: bool,
    /// Installed geometry contains no point domains; omit compact hit decoding.
    pub(crate) no_points: bool,
    pub(crate) no_boundary_transactions: bool,
    /// The irradiance volume (probes.slang) replaces world diffuse light.
    pub(crate) probes: bool,
    /// Raster physical-fibre resources participate in pipeline layout identity.
    pub(crate) fibre: bool,
    pub(crate) denoise: bool,
    pub(crate) field_controls: bool,
    pub(crate) bounded_secondary_compilation: bool,
    pub(crate) medium: Option<Arc<crate::medium::Native>>,
}

pub(crate) struct SlangForward {
    device: wgpu::Device,
    metal: bool,
    check_only: bool,
    ray_shadows: bool,
    msaa: bool,
    /// The camera vertex stage, shared by depth and forward (bit-identical depth).
    camera: wgpu::ShaderModule,
    opaque_depth: wgpu::ShaderModule,
    single_depth: [wgpu::ShaderModule; 2],
    /// Alpha-tested depth: cutout, then hashed (dithered) coverage.
    masked_depth: [wgpu::ShaderModule; 2],
    /// Decoupled visibility keys (decoupled_visibility.slang): vertex, double- and
    /// single-sided fragment, the alpha-tested vertex and fragment stages, then
    /// transparent draws' fragment lists.
    visibility: [wgpu::ShaderModule; 6],
    /// Shared geometry vertex stage for fibre captures.
    fibre_vertex: wgpu::ShaderModule,

    authored_coverage: Mutex<std::collections::HashMap<(u32,crate::pipelines::CoverageStage),Arc<OnceLock<wgpu::ShaderModule>>>>,
    fibre: [OnceLock<wgpu::ShaderModule>;crate::fibre_transport::Stage::ALL.len()],
    scene: SlangScene,
    authored_controls: Mutex<std::collections::HashMap<(ForwardKey,u32),Arc<OnceLock<wgpu::ShaderModule>>>>,
    /// The precompiled generic per fragment Stage (Stage::ALL), without and
    /// with hashed coverage (ForwardKey::generic).
    generic: [[OnceLock<wgpu::ShaderModule>; 2]; Stage::ALL.len()],
    /// Disk-cache path for a variant's native code, by its module text.
    cache: Box<dyn Fn(blake3::Hash) -> PathBuf + Send + Sync>,
}

impl SlangForward {
    pub(crate) fn new(
        r: &crate::Renderer,
        ray_shadows: bool,
        msaa: bool,
        scene: SlangScene,
    ) -> Self {
        let store = r.disk_cache.clone();
        let mut this = Self {
            device: r.device.clone(),
            check_only: r.check_only,
            metal: r.adapter_info.backend == wgpu::Backend::Metal,
            ray_shadows,
            msaa,
            camera: native::shader_module(&r.device, &native::DEPTH_MAIN_VS),
            opaque_depth: native::shader_module(&r.device, &native::DEPTH_OPAQUE_DEPTH_VS),
            single_depth: [
                native::shader_module(&r.device, &native::DEPTH_OPAQUE_SINGLE_DEPTH_VS),
                native::shader_module(&r.device, &native::DEPTH_OPAQUE_SINGLE_DEPTH_FS),
            ],
            masked_depth: [
                native::shader_module(&r.device, &native::DEPTH_DEPTH_FS),
                native::shader_module(&r.device, &native::DEPTH_HASHED_DEPTH_FS),
            ],
            visibility: [
                native::shader_module(&r.device, &native::DECOUPLED_VISIBILITY_VS),
                native::shader_module(&r.device, &native::DECOUPLED_VISIBILITY_FS),
                native::shader_module(&r.device, &native::DECOUPLED_VISIBILITY_SINGLE_FS),
                native::shader_module(&r.device, &native::DECOUPLED_VISIBILITY_MASKED_VS),
                native::shader_module(&r.device, &native::DECOUPLED_VISIBILITY_MASKED_FS),
                native::shader_module(&r.device, if scene.field_controls {&native::DECOUPLED_VISIBILITY_TRANSPARENT_FRAGMENTS_FS} else {&native::DECOUPLED_VISIBILITY_TRANSPARENT_TRIANGLES_FS}),
            ],
            fibre_vertex: native::shader_module(&r.device,&native::DEPTH_SHADOW_VS),
            authored_coverage:Default::default(),
            fibre:Default::default(),
            scene,
            authored_controls:Default::default(),
            generic: Default::default(),
            cache: Box::new(move |hash| store.path("slang-variant", 1, hash)),
        };
        let attributes=this.scene.attributes;
        if attributes > 0 {
            let stream = |module, entry| {
                if let Some(&(_,_,_,shader))=native::STREAMS.iter().find(|&&(a,m,e,_)| a==attributes && m==module && e==entry) {
                    native::shader_module(&this.device,shader)
                } else {
                    this.compile_entry(module,None,(entry,if entry.ends_with("vs") {"vertex"} else {"fragment"}),(0,0,0),entry,None).expect("compile scene attribute stream")
                }
            };
            let camera=stream("depth","main_vs");
            let fibre_vertex=stream("depth","shadow_vs");
            let masked_depth=[stream("depth","depth_fs"),stream("depth","hashed_depth_fs")];
            let masked_vs=stream("decoupled_visibility","masked_vs");
            let transparent_fs=stream("decoupled_visibility",if this.scene.field_controls {"transparent_fragments_fs"} else {"transparent_triangles_fs"});
            let masked_fs=stream("decoupled_visibility","masked_fs");
            this.camera=camera;this.fibre_vertex=fibre_vertex;this.masked_depth=masked_depth;
            this.visibility[3]=masked_vs;this.visibility[5]=transparent_fs;
            this.visibility[4]=masked_fs;

        }
        this
    }
    fn generic(&self,stage:Stage,hashed:bool)->wgpu::ShaderModule {
        self.generic[stage as usize][usize::from(hashed)].get_or_init(|| {
            if let Some(&(_,_,_,_,shader))=native::CONTROL_GENERICS.iter()
                .find(|&&(a,d,s,h,_)|!self.scene.field_controls && a==self.scene.attributes && d==self.scene.denoise && s==stage as usize && h==hashed) {
                native::shader_module(&self.device,shader)
            } else {
                let key=ForwardKey::generic(stage,hashed);
                self.compile_entry("scene_controls",Some(&key.module_with(None)),key.entry(),(0,0,0),key.entry().0,None).expect("compile scene attribute controls")
            }
        }).clone()
    }

    /// A coverage stage with the scene's programs: masked depth, masked shadow, or
    /// alpha-tested visibility keys (surface.slang program_covers).
    fn program_shadow(&self, entry: &str, program:u32) -> anyhow::Result<wgpu::ShaderModule> {
        let programs=&self.scene.programs.as_ref().unwrap().camera[program as usize-1];
        let (visibility,ret,body)=match entry {
            "depth_fs" => ("","void","v.primitive=primitive;let s=surface<Unspecialized,Programs>(v,front);if(s.discarded){discard;}"),
            "masked_fs" => ("import decoupled_fragments;","VisibilityOutput","return masked_visibility<Programs>(v,front,primitive);"),
            _ => unreachable!("unsupported coverage entry"),
        };
        let target=if matches!(ret,"void"|"VisibilityOutput") {""} else {" : SV_Target0"};
        let text=format!("import vork_constants;import frame;import stream;import material;import surface;import variant;import vork_shadow_coverage;{visibility}\n{}\n[shader(\"fragment\")] {ret} {entry}(Varying v,bool front:SV_IsFrontFace,uint primitive:SV_PrimitiveID){target} {{{body}}}",programs.code);
        let text=if self.scene.program_trace {format!("import vork_rays;\n{text}")} else {text};
        self.compile_entry_for("program_depth",Some(&text),(entry,"fragment"),(0,0,0),entry,None,Some(program))
            .with_context(|| format!("compile material program {} coverage stage {entry}",programs.name))
    }

    /// Compile `entry` of module `main` (generated `text`, or a renderer module)
    /// with the scene's programs and attributes, through the disk cache. A
    /// compute entry's `threads` is its threadgroup size (Metal's passthrough
    /// module carries it); render stages pass zero.
    fn compile_entry(
        &self,
        main: &str,
        text: Option<&str>,
        entry: (&str, &str),
        threads: (u32, u32, u32),
        label: &str,
        kernel: Option<crate::pipelines::ProgramKernel>,
    ) -> anyhow::Result<wgpu::ShaderModule> {
        self.compile_entry_for(main,text,entry,threads,label,kernel,None)
    }

    fn compile_entry_for(
        &self, main:&str, text:Option<&str>, entry:(&str,&str), threads:(u32,u32,u32), label:&str,
        kernel:Option<crate::pipelines::ProgramKernel>, program:Option<u32>,
    )->anyhow::Result<wgpu::ShaderModule> {
        Ok(self.entry_module(main,text,entry,threads,label,kernel,program,false)?.built())
    }

    /// `compile_entry_for`, or with `probe` only whether the store already holds
    /// its compiled code (no Slang compile, no module).
    #[allow(clippy::too_many_arguments)]
    fn entry_module(
        &self, main:&str, text:Option<&str>, entry:(&str,&str), threads:(u32,u32,u32), label:&str,
        kernel:Option<crate::pipelines::ProgramKernel>, program:Option<u32>, probe:bool,
    )->anyhow::Result<Lookup<wgpu::ShaderModule>> {
        // Authored stages and their effect proofs were validated before this
        // backend link. The check-only renderer cannot execute this module and
        // never writes a native artifact or marks a cache entry as compiled.
        if self.check_only {
            return Ok(if probe { Lookup::Stored } else {
                Lookup::Built(native::module(&self.device, label, None, Some(""), threads))
            });
        }
        use crate::pipelines::{ProgramKernel, ShadeStage};
        let diagnostic=kernel.and_then(ProgramKernel::camera_label);
        let label=diagnostic.as_deref().unwrap_or(label);
        let (_, target, flags) = if self.metal {
            METAL_TARGET
        } else {
            SPIRV_TARGET
        };
        let all_programs = self.table_programs(kernel);
        let program=program.or_else(||if let Some(ProgramKernel::Decoupled {stage:ShadeStage::EvaluateProgram(id),..})=kernel {Some(id)} else {None});
        let primary=program.and_then(|id|all_programs.map(|p|&p.camera[id as usize-1]));
        let programs=primary.or(all_programs);
        let path_medium=self.scene.medium.as_ref().filter(|_|matches!(kernel,Some(ProgramKernel::Path {media:Some(_),..})));
        let mut kernel_defines=vec![format!("-DVORK_AOV_COUNT={}",self.scene.aov_count),format!("-DVORK_SHADOW_CAPACITY={}",self.scene.shadow_capacity)];
        if self.scene.program_trace {kernel_defines.push("-DVORK_PROGRAM_TRACE".into());}
        if self.scene.program_trace && main=="program_depth" && entry.0=="masked_fs" {
            kernel_defines.push("-DVORK_RAY_BINDING_GROUP=4".into());
            if self.metal {
                // Global/material buffers, fragment storage, and four fragment
                // geometry buffers precede visibility's retained shadow group.
                kernel_defines.extend(["-DVORK_RAY_SEQUENCE_REGISTER=t15","-DVORK_RAY_SCENE_REGISTER=t16","-DVORK_RAY_ALPHA_REGISTER=t17","-DVORK_RAY_HITS_REGISTER=b18"].map(str::to_owned));
            }
        }
        if !self.scene.field_controls && !matches!(kernel,Some(ProgramKernel::Path {..})) {kernel_defines.push("-DVORK_NO_IMPLICIT".into());}
        if self.scene.no_points {kernel_defines.push("-DVORK_NO_POINTS".into());}
        if self.scene.no_shadow_containment {kernel_defines.push("-DVORK_NO_SHADOW_CONTAINMENT".into());}
        if self.scene.no_boundary_transactions && matches!(kernel,Some(ProgramKernel::Path {..})) {kernel_defines.push("-DVORK_NO_BOUNDARY_TRANSACTIONS".into());}
        if !self.scene.connection_transmission {kernel_defines.push("-DVORK_NO_CONNECTION_TRANSMISSION".into());}
        // Medium kernels already have their own outlining policy. Combining it
        // with field outlining makes the NVIDIA driver expand nested closures.
        if self.scene.field_controls && self.scene.medium.is_none() {kernel_defines.push("-DVORK_FIELD_OUTLINE".into());}
        if self.scene.field_controls && matches!(entry.0,"controls_fs"|"transparent_controls_fs") {kernel_defines.push("-DVORK_IMPLICIT_CONTROLS".into());}
        #[cfg(any(test, feature="test-support"))]
        if matches!(kernel,Some(ProgramKernel::Path {..})) {
            if let Ok(mode)=std::env::var("VORK_TEST_POISON_PATH_SAMPLE") {
                kernel_defines.push(format!("-DVORK_TEST_POISON_PATH_SAMPLE={}",if mode=="all" {2}else{1}));
            }
        }
        if let Some(ProgramKernel::Path {media:Some(count),..})=kernel {
            let native=path_medium.expect("installed path medium");
            if native.convex {
                kernel_defines.push("-DVORK_MEDIUM_CONVEX".to_owned());
            }
            if let Some(limit)=native.work_limit {kernel_defines.push(format!("-DVORK_MEDIUM_WORK_LIMIT={limit}"));}
            if !native.homogeneous && native.scattering_free && native.emission_free {kernel_defines.push("-DVORK_MEDIUM_ABSORPTION_ONLY".to_owned());}
            kernel_defines.extend(["-DVORK_PATH_MEDIA".to_owned(),format!("-DVORK_PATH_MEDIUM_BOUNDS={count}"),format!("-DVORK_PATH_HETEROGENEOUS={}",u8::from(!native.homogeneous))]);
        }
        if let Some(ProgramKernel::Decoupled {stage,features,boundaries,..})=kernel {
            kernel_defines.extend(crate::camera_native::COMPILER_FLAGS.iter().map(|flag|(*flag).to_owned()));
            kernel_defines.push(format!("-DVORK_REFRACTION_BOUNDARIES={boundaries}"));
            kernel_defines.push("-DVORK_CAMERA_SPLIT".into());
            if matches!(stage,ShadeStage::Fused|ShadeStage::Compose) {kernel_defines.push("-DVORK_COMPOSE_STORED".into());}
            if features & crate::forward_variant::FEATURE_RECEIVER_DIFFUSION!=0 {kernel_defines.push("-DVORK_RECEIVER_DIFFUSION".into());}
            if stage==ShadeStage::Fused {kernel_defines.push("-DVORK_CAMERA_FUSED".into());}
            if matches!(stage,ShadeStage::Index|ShadeStage::Classify|ShadeStage::Prefix|ShadeStage::Scatter) {kernel_defines.push("-DVORK_CAMERA_CLASSIFY".into());}
            if stage==ShadeStage::Compose {kernel_defines.push(NATIVE_OPTIMIZATION.to_owned());}
            if stage.composes() && self.msaa {kernel_defines.push("-DVORK_RASTER_MSAA".into());}
        }

        // Preserve final's compiler inputs: O2 changes secondary sample results.
        // Draft permits the bounded material/lighting graphs that admit large scenes.
        let surface_outlining = !self.metal && !self.scene.field_controls && self.scene.medium.is_none()
            && (matches!(kernel, Some(ProgramKernel::Gi {..}))
                || (self.scene.bounded_secondary_compilation && matches!(kernel, Some(ProgramKernel::Reflections {..}))));
        if surface_outlining {kernel_defines.push("-DVORK_SURFACE_SLANG_OUTLINE".into());}
        let transport_outlining = matches!(kernel, Some(ProgramKernel::Path {..}))
            || (surface_outlining && self.scene.bounded_secondary_compilation);
        // Camera kernels otherwise duplicate the implicit grid walk at every
        // classify/evaluate call site. Keep their existing Slang boundaries.
        let camera_outlining = !self.metal && matches!(kernel, Some(ProgramKernel::Decoupled {..}));
        if transport_outlining {
            kernel_defines.extend([NATIVE_OPTIMIZATION.to_owned(), "-DVORK_PATH_SLANG_OUTLINE".into()]);
        }
        if path_medium.is_some() {
            kernel_defines.extend(vork_slang::MEDIUM_FP_FLAGS.map(str::to_owned));
        }
        let adaptive=matches!(kernel,Some(ProgramKernel::Path {adaptive:true,..}));
        let mut program_modules = programs.map_or_else(Vec::new, |p| p.modules.clone());
        if matches!(kernel,Some(ProgramKernel::Path {..})) {
            if let Some(p)=&self.scene.propagation {
                kernel_defines.push("-DVORK_PROPAGATION".into());
                for module in &p.modules {
                    if !program_modules.iter().any(|(name,_)|name==&module.0) {program_modules.push(module.clone());}
                }
            }
        }
        if matches!(kernel,Some(ProgramKernel::Path {..})) {
            if let Some(camera)=&self.scene.camera {
                kernel_defines.push("-DVORK_LIGHT_PATHS".into());
                for module in &camera.modules {
                    if !program_modules.iter().any(|(name,_)|name==&module.0) {program_modules.push(module.clone());}
                }
            }
        }

        if let Some(environment)=&self.scene.environment {
            kernel_defines.push("-DVORK_SPATIAL_ENVIRONMENT".into());
            for module in &environment.modules {
                if !program_modules.iter().any(|(name,_)|name==&module.0) {program_modules.push(module.clone());}
            }
        }
        let secondary=entry.1=="compute" && (self.scene.ray_refraction || self.ray_shadows && (self.scene.connection_transmission || self.scene.attribute_coverage || all_programs.is_some_and(|p|p.coverage || self.scene.fibre)));
        if primary.is_some() && secondary {
            for module in &all_programs.unwrap().modules {
                if let Some(existing)=program_modules.iter_mut().find(|(name,_)|name==&module.0) {*existing=module.clone();} else {program_modules.push(module.clone());}
            }
        }
        // Secondary camera transport already receives CameraPrograms as P.
        // Shadow/probe candidate callbacks still import their shared dispatcher.
        let medium_sources=self.scene.medium.as_ref().is_some_and(|m|m.mesh_sources);
        let program_coverage=secondary && self.ray_shadows && (medium_sources || self.scene.connection_transmission || self.scene.attribute_coverage || all_programs.is_some_and(|p|p.coverage || self.scene.fibre));
        if let Some(p)=all_programs.filter(|_|program_coverage) {
            let p=if !self.scene.connection_transmission && !medium_sources {p.for_coverage()} else {p};
            program_modules.push(("scene_programs".into(),p.code.replace("struct Programs :","struct ScenePrograms :")));
        }
        if program_coverage && all_programs.is_none() {
            program_modules.push(("scene_programs".into(), "import surface; typedef NoPrograms ScenePrograms;".into()));
        }
        if let Some(native)=path_medium {
            if programs.is_none() {program_modules.extend(programs::abi().native_modules());}
            program_modules.extend(native.modules.clone());
            program_modules.push(("path_medium_scene".into(),format!("import frame;\n{}",native.point)));
        }
        let attributes=if main=="decoupled_visibility" && matches!(entry.0,"fs"|"single_fs") {0} else {self.scene.attributes};
        let define = format!("-DVORK_ATTRIBUTES={attributes}");
        // Controls do not produce lighting; only compute consumers read probes.
        let probes = self.scene.probes && entry.1 == "compute";
        // Secondary compute kernels own group 2; decoupled colour shares the
        // raster shadow group and reads the volume after its shadow entries.
        let fibre = self.scene.fibre && main == "decoupled_programs";
        let fibre_binding="-DVORK_FIBRE_BINDING=4";
        let probe_binding = format!("-DVORK_PROBE_BINDING={}", if main == "decoupled_programs" { 9 } else { 0 });
        // Decoupled frame, sampling and record buffers precede probes.
        let probe_register: Option<u32> = if self.metal && probes {
            match main { "decoupled_programs" => Some(15), _ => None }
        } else {None};
        let denoise=self.scene.denoise && matches!(entry.0,"controls_fs"|"transparent_controls_fs");
        let mut modules = native::SOURCES.to_vec();
        for (name, source) in &program_modules {
            if let Some(module)=modules.iter_mut().find(|(n,_)|*n==name) {*module=(name,source);} else {modules.push((name,source));}
        }
        if let Some(text) = text {
            modules.push((main, text));
        }
        let mut include: Vec<String> = if secondary {all_programs} else {programs}
            .map_or(&[][..], |p| &p.include[..])
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        if let Some(environment)=&self.scene.environment {include.extend(environment.include.iter().map(|p|p.display().to_string()));}
        if let Some(p)=&self.scene.propagation {include.extend(p.include.iter().map(|p|p.display().to_string()));}
        if let Some(camera)=&self.scene.camera {include.extend(camera.include.iter().map(|p|p.display().to_string()));}
        if let Some(native)=path_medium {include.extend(native.includes.iter().map(|p|p.display().to_string()));}
        let query_defines:Vec<_> = if self.metal && entry.1=="vertex" {
            vec!["-DVORK_MATERIAL_WORDS_REGISTER=t3","-DVORK_FRAME_LIGHTS_REGISTER=t30","-DVORK_FRAME_CLUSTERS_REGISTER=b29","-DVORK_POINT_WORDS_REGISTER=t2"]
        }else{Vec::new()};
        let kernel_module = match main {
            "gi_programs" => "gi_ray",
            "reflections_programs" => "reflections_hits", "probe_programs" => "probe_update",
            "path_programs_main" | "path_programs_fixed" => "path", _ => "",
        };
        let mut native_registers = if self.metal {
            crate::forward_variant::hit_kernel_registers(kernel_module,
                adaptive,
                main == "probe_programs" || (self.scene.probes && matches!(main,"gi_programs"|"reflections_programs")))
        } else {Vec::new()};
        if let Some(first) = probe_register {
            native_registers.extend(crate::forward_variant::probe_registers(first));
        }
        let args: Vec<&str> = ["-target", target]
            .into_iter()
            .chain(SLANG_FLAGS.iter().copied())
            .chain(flags.iter().copied())
            .chain(native_registers.iter().map(String::as_str))
            .chain(program_coverage.then_some("-DVORK_PROGRAM_COVERAGE"))
            .chain(self.scene.attribute_coverage.then_some("-DVORK_ATTRIBUTE_COVERAGE"))
            .chain(kernel_defines.iter().map(String::as_str))
            .chain(query_defines.iter().copied())

            .chain((self.ray_shadows || (main=="decoupled_programs" && self.scene.ray_refraction)).then_some("-DVORK_RAY_BINDINGS"))
            .chain((main=="decoupled_programs" && self.scene.ray_refraction).then_some("-DVORK_RAY_REFRACTION"))
            .chain(denoise.then_some("-DVORK_DENOISE"))
            .chain(probes.then_some("-DVORK_PROBES"))
            .chain(fibre.then_some("-DVORK_FIBRE"))
            .chain(fibre.then_some(fibre_binding))
            .chain(probes.then_some(probe_binding.as_str()))
            .chain((self.scene.attributes > 0).then_some(define.as_str()))
            .chain(include.iter().flat_map(|d| ["-I", d.as_str()]))
            .collect();
        let compiler=vork_slang::Compiler::discover();
        let identities=(if secondary {all_programs} else {programs}).map(|p|p.identities.clone()).unwrap_or_default();
        let inputs = (if secondary {all_programs} else {programs}).map(|p| p.inputs.as_slice()).unwrap_or(&[]);
        let (identity,prepared)=if probe {
            let Some(identity)=programs::cached_native_input_identity(self.cache.as_ref(),&compiler,&modules,main,&[entry],&args,&identities,inputs)? else {return Ok(Lookup::Absent);};
            (identity,None)
        } else {
            programs::native_input_identity(self.cache.as_ref(),&compiler,&modules,main,&[entry],&args,&identities,inputs)?
        };
        let mut hash = blake3::Hasher::new();
        if transport_outlining && !self.metal {hash.update(b"path-local-blend-state-v1");}
        if self.scene.field_controls {hash.update(b"field-driver-inline-v1");}
        if camera_outlining {hash.update(b"camera-driver-outline-v1");}
        hash.update(&[u8::from(self.ray_shadows),u8::from(self.msaa),u8::from(denoise),u8::from(main=="decoupled_programs" && self.scene.ray_refraction),u8::from(self.scene.connection_transmission)]);
        for part in [target, main, entry.0, &define, text.unwrap_or("")] {
            hash.update(part.as_bytes());
            hash.update(&[0]);
        }
        for option in &kernel_defines {hash.update(option.as_bytes());hash.update(&[0]);}
        hash.update(&[u8::from(adaptive)]);
        if probes {
            hash.update(probe_binding.as_bytes());
            if let Some(first) = probe_register {hash.update(&first.to_le_bytes());}
        }
        if fibre {hash.update(fibre_binding.as_bytes());}
        // Embedded renderer modules and compiler inputs have a shader-only
        // content identity in the store, independent of host builds/devices.
        let prefix = hash.clone();
        hash.update(&identity.identity);
        let path = (self.cache)(hash.finalize());
        if probe {return Ok(if disk_cache::read(&path).is_some() {Lookup::Stored} else {Lookup::Absent});}
        let receipt_path = programs::native_input_receipt_path(self.cache.as_ref(), &compiler, &modules, main, &[entry], &args, &identities, inputs)?;
        let code = programs::native_code(&identity, prepared, inputs, &receipt_path, |identity| {
            let mut hash = prefix.clone();
            hash.update(identity);
            (self.cache)(hash.finalize())
        }, || compiler.prepare_identified_modules(&modules, main, &[entry], &args, &identities), |prepared| {
            let start = std::time::Instant::now();
            let mut code = prepared.compile()?;
            crate::timing::creation(&self.device,label,"Slang",start.elapsed().as_secs_f64()*1000.,0.);
            if transport_outlining && !self.metal {
                let start = std::time::Instant::now();
                code=compiler.localize_native_blends(&code)?;
                crate::timing::creation(&self.device,label,"SPIR-V optimization",start.elapsed().as_secs_f64()*1000.,0.);
            }
            if self.metal && matches!(kernel,Some(ProgramKernel::Decoupled {stage:ShadeStage::Evaluate,..})) {
                code=crate::camera_native::inline_metal_functions(&code);
            }
            if (self.scene.field_controls || surface_outlining || transport_outlining) && !self.metal && !camera_outlining { allow_driver_inlining(&mut code)?; }
            Ok(code)
        })?;
        Ok(Lookup::Built(if self.metal {
            let source=String::from_utf8(code)?;
            vork_slang::metal_source::validate(&source)?;
            native::module(&self.device, label, None, Some(&source), threads)
        } else {
            native::module(&self.device, label, Some(&code), None, (0, 0, 0))
        }))
    }

    fn camera_shipped(&self,stage:crate::pipelines::ShadeStage,receiver_diffusion:bool)->Option<&'static native::Compiled> {
        // Shipped camera kernels specialize for triangle-only scenes.
        if self.scene.field_controls {return None;}
        use crate::pipelines::ShadeStage;
        if self.scene.attributes>crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS as u32
            && !matches!(stage,ShadeStage::Index|ShadeStage::Classify|ShadeStage::Prefix|ShadeStage::Scatter) {return None;}
        if self.scene.environment.is_some() && stage.composes() {return None;}
        // Shipped evaluators read proven World profiles through the retained words.
        // Other domains and coefficient programs still link their transport.
        // Coverage needs the runtime camera payload contract for every producer
        // and consumer, including native specialized evaluators and composition.
        // Shipped camera records carry no AOVs. Classification has no surface
        // payload; every evaluator and consumer of that payload must agree on N.
        if (self.scene.aov_count!=0 || self.scene.surface_coverage) && matches!(stage,ShadeStage::EvaluateOpaque|ShadeStage::Evaluate|ShadeStage::Compose) {return None;}
        let name=match stage {
            ShadeStage::Index=>"index",ShadeStage::Classify=>"classify",ShadeStage::Prefix=>"prefix",ShadeStage::Scatter=>"scatter",
            ShadeStage::EvaluateOpaque=>"evaluate_opaque",ShadeStage::Evaluate=>"evaluate",ShadeStage::EvaluateProgram(_)|ShadeStage::Fused|ShadeStage::Occupancy|ShadeStage::Indirect=>return None,
            ShadeStage::Compose=>if receiver_diffusion {"combine_diffusion"} else {"combine"},
        };
        Some(native::CAMERAS.iter().find(|(m,r,n,_)|*m==u8::from(self.msaa && stage.composes()) && *r==u8::from(self.ray_shadows) && *n==name).unwrap().3)
    }
    /// Classification/composition are embedded; material evaluation specializes at install.
    fn camera_fixed(&self,kernel:crate::pipelines::ProgramKernel)->Option<&'static native::Compiled> {
        use crate::pipelines::{ProgramKernel,ShadeStage};
        let ProgramKernel::Decoupled {stage,features,..}=kernel else {return None;};
        if stage.table() || matches!(stage,ShadeStage::EvaluateProgram(_)|ShadeStage::Indirect)
            {return None;}
        self.camera_shipped(stage,features & crate::forward_variant::FEATURE_RECEIVER_DIFFUSION!=0)
    }

    fn surface_fixed(&self,kernel:crate::pipelines::ProgramKernel)->Option<(&'static native::Compiled,Vec<(&'static str,crate::forward_variant::Capability)>)> {
        use crate::pipelines::{ProgramKernel,ShadeStage};
        use crate::surface_native::SurfaceKernel;
        if self.scene.surface_coverage && matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {..}) {return None;}
        if self.metal || self.scene.field_controls || self.scene.attributes!=0 || self.scene.aov_count!=0
            || self.scene.environment.is_some() || self.scene.fibre
            || !self.scene.no_shadow_containment || self.scene.connection_transmission && (self.scene.programs.is_some() || !self.ray_shadows)
            || self.table_programs(Some(kernel)).is_some() {return None;}
        let boundaries=match kernel {ProgramKernel::Decoupled {boundaries,..}=>boundaries,_=>0};
        // Shared transport has no authored medium continuation. Keep those links
        // on their existing runtime path even when the active set fits one slot.
        if boundaries>1 || self.scene.ray_refraction && self.scene.medium.is_some() {return None;}
        // Setup warms refraction with probes. Keep non-probe refraction on its
        // established runtime executable: late-specialized native arithmetic can
        // perturb narrow glass highlights before denoising in that configuration.
        if self.scene.ray_refraction && !self.scene.probes {return None;}
        // Full candidate evaluation reads hit records installed for transmission.
        // No containment alone does not provide those records: an opaque scene
        // can have only compact alpha entries, whose hit address is NO_ALPHA.
        // Caster links force even opaque geometry through the candidate reader.
        let coverage=self.scene.connection_transmission && self.scene.no_shadow_containment && self.ray_shadows;
        let (family,features,ray_shadows,subsurface,emitter_profiles)=match kernel {
            // The existing fixed probe entry owns the unspecialized zero key.
            ProgramKernel::Probes {features:0,..}=>return None,
            ProgramKernel::Gi {features} if self.scene.medium.is_none()=>(SurfaceKernel::Gi,features,false,false,false),
            ProgramKernel::Reflections {features} if self.scene.medium.is_none()=>(SurfaceKernel::Reflections,features,false,false,false),
            ProgramKernel::Probes {features,directional:true} if self.scene.medium.is_none()=>(SurfaceKernel::Directional,features,false,false,false),
            ProgramKernel::Probes {features,directional:false} if self.scene.medium.is_none()=>(SurfaceKernel::Probes,features,false,false,false),
            ProgramKernel::Decoupled {stage:ShadeStage::Evaluate|ShadeStage::EvaluateOpaque,features,subsurface,emitter_profiles,..}=>(SurfaceKernel::Evaluate,features,self.ray_shadows,subsurface,emitter_profiles),
            ProgramKernel::Decoupled {stage:ShadeStage::Indirect,features,subsurface,emitter_profiles,..}
                if self.scene.medium.is_none() && self.scene.probes && self.ray_shadows=>(SurfaceKernel::Indirect,features,self.ray_shadows,subsurface,emitter_profiles),
            ProgramKernel::Decoupled {stage:ShadeStage::Occupancy,features,subsurface,emitter_profiles,boundaries:1}
                if coverage=>(SurfaceKernel::Occupancy,features,self.ray_shadows,subsurface,emitter_profiles),
            _=>return None,
        };
        if subsurface && !self.ray_shadows {return None;}
        let shader=native::SURFACES.iter().find(|(f,r,p,c,b,_)|*f==family && *r==(family.camera() && self.ray_shadows) && *p==self.scene.probes && *c==coverage && *b==boundaries).expect("shipped surface resource interface").5;
        Some((shader,crate::forward_variant::variant_capabilities(features,ray_shadows,subsurface,emitter_profiles)))
    }

    /// Each program owns its evaluation bin, so table slots hold only built-in
    /// materials: table camera stages take the scene's programs only when secondary
    /// transport (ray refraction, program shadow coverage, fibres) can reach one.
    /// Then no table kernel grows with the scene's program count.
    fn table_programs(&self,kernel:Option<crate::pipelines::ProgramKernel>)->Option<&programs::Native> {
        // Composition consumes stored closures and explicitly calls NoPrograms.
        // Do not import the scene dispatcher merely to define an unused type.
        if matches!(kernel,Some(crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Compose,..})) {return None;}
        let table=matches!(kernel,Some(crate::pipelines::ProgramKernel::Decoupled {stage,..}) if stage.table());
        self.scene.programs.as_deref().filter(|_|!table || self.transport_programs())
    }
    fn transport_programs(&self)->bool {
        self.scene.programs.as_ref().is_some_and(|p|self.scene.ray_refraction || self.ray_shadows && (p.coverage || self.scene.connection_transmission) || self.scene.fibre)
    }

    fn camera_specialized(&self,kernel:crate::pipelines::ProgramKernel,probe:bool)->anyhow::Result<Lookup<(wgpu::ShaderModule,&'static str)>> {
        use crate::pipelines::{ProgramKernel,ShadeStage};
        if let ProgramKernel::Decoupled {stage,..}=kernel {
            let fixed=match stage {
                ShadeStage::Index=>Some(("index","camera_index")),
                ShadeStage::Classify=>Some(("classify","camera_classify")),
                ShadeStage::Prefix=>Some(("prefix","camera_prefix")),
                ShadeStage::Scatter=>Some(("scatter","camera_scatter")),
                _=>None,
            };
            if let Some((stage,entry))=fixed {
                let source=crate::camera_native::entry_source(stage);
                let module=self.entry_module("camera_field_classification",Some(&source),(entry,"compute"),(8,8,1),entry,Some(kernel),None,probe)?;
                return Ok(module.map(|m|(m,entry)));
            }
        }
        let source=self.table_programs(Some(kernel));
        let primary=if let crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::EvaluateProgram(id),..}=kernel {source.map(|source|&source.camera[id as usize-1])} else {None};
        let code=primary.or(source).map_or_else(String::new,|source|source.code.replace("struct Programs :","struct CameraPrograms :"));
        let secondary=source.filter(|_|primary.is_some() && self.scene.ray_refraction).map_or_else(String::new,|source|source.code.clone());
        let program=if source.is_some() {"CameraPrograms"} else {"NoPrograms"};
        let transport=if secondary.is_empty() {program} else {"Programs"};
        let decode_table=!matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::EvaluateProgram(_),..});
        let fused=matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Fused,..});
        let compose=matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Compose,..});
        let occupancy=matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Occupancy,..});
        let indirect=matches!(kernel,crate::pipelines::ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Indirect,..});
        let (threads,call)=if occupancy {
            ((8,8,1),format!("prepare_refraction_occupancy<Key,{transport}>(gid);"))
        } else if indirect {
            ((vork_slang::registry::CAMERA_INDIRECT_GROUP,1,1),format!("indirect_material<Key,{program}>(gid);"))
        } else if compose {
            ((8,8,1),"if(camera_config.screen!=0u && camera_config.lit!=0u) {composite_pixel<Key,NoPrograms,CameraTransmittance,3>(gid);} else {shade_pixel<Key,NoPrograms,CameraTransmittance,CameraDiffusion,3>(gid);}".to_owned())
        } else if fused {
            ((8,8,1),"shade_pixel<Key,NoPrograms,CameraTransmittance,CameraDiffusion,3>(gid);".to_owned())
        } else {
            ((64,1,1),format!("evaluate_material<Key,{program},{decode_table},{transport}>(gid);"))
        };
        let text=format!("import camera_config;import decoupled;import surface;import frame;\n{secondary}\n{code}\n[shader(\"compute\")][numthreads({},{},{})] void decoupled_programs(uint3 gid:SV_DispatchThreadID) {{{call}}}",threads.0,threads.1,threads.2);
        let text={
            let crate::pipelines::ProgramKernel::Decoupled {features,subsurface,emitter_profiles,..}=kernel else {unreachable!("camera program key")};
            crate::forward_variant::variant_module(features,self.ray_shadows,subsurface,emitter_profiles,None,&text)
        };
        let module=self.entry_module("decoupled_programs",Some(&text),("decoupled_programs","compute"),threads,"camera program",Some(kernel),None,probe)
            .with_context(|| format!("compile camera stage {} with material program {}",kernel.camera_label().unwrap_or_else(||"decoupled_programs".into()),
                primary.map_or("table",|p|p.name.as_str())))?;
        Ok(module.map(|module|(module,"decoupled_programs")))
    }

    fn program_controls(&self,stage:Stage,features:FeatureMask,program:u32)->anyhow::Result<[wgpu::ShaderModule;2]> {
        let key=ForwardKey::new(stage,features);
        let slot=self.authored_controls.lock().unwrap().entry((key,program)).or_default().clone();
        let fragment=try_init(&slot,|| {
            let source=&self.scene.programs.as_ref().unwrap().camera[program as usize-1];
            let text=key.module_with(Some(&source.code));
            self.compile_entry_for(&key.name(),Some(&text),key.entry(),(0,0,0),&key.name(),None,Some(program))
                .with_context(|| format!("compile material program {} controls stage {}",source.name,key.name()))
        })?;
        Ok([self.camera.clone(),fragment.clone()])
    }

    fn table_controls(&self,stage:Stage,hashed:bool)->[wgpu::ShaderModule;2] {
        [self.camera.clone(),self.generic(stage,hashed)]
    }

}

impl VariantShader for SlangForward {
    fn shared_kernel_key(&self, kernel: crate::pipelines::ProgramKernel) -> Option<blake3::Hash> {
        let (shader, capabilities) = self.surface_fixed(kernel)?;
        // Retain descriptor state: drivers may bake array offsets into machine
        // code even for runtime-sized SPIR-V arrays. Display/control settings and
        // camera programs do not affect this shipped executable.
        if self.scene.medium.is_some() { return None; }
        Some(blake3::hash(format!("surface executable {} {capabilities:?} {} {} {}",
            shader.name,self.msaa,self.scene.textures.max(vork_slang::registry::MATERIAL_TEXTURE_CAPACITY),self.scene.samplers.max(vork_slang::registry::MATERIAL_SAMPLER_CAPACITY)).as_bytes()))
    }
    fn generic_shipped(&self) -> bool { self.scene.programs.is_none() }
    fn modules(&self, key: GeometryKey) -> anyhow::Result<[wgpu::ShaderModule; 2]> {
        Ok(match key.1 {
            Role::Fibre(stage) => {
                let index=stage.index();
                let fragment=try_init(&self.fibre[index],|| {
                    let programs=self.scene.programs.as_ref().expect("fibre entry needs compiled hair programs");
                    let entry=stage.entry();
                    let body:String=match stage {
                        crate::fibre_transport::Stage::Accumulate=>"return fibre_accumulate<Programs>(v,front);".into(),
                        crate::fibre_transport::Stage::Near=>"fibre_depth<Programs>(v,front);return uint2(v.object,v.curve_strand);".into(),
                        crate::fibre_transport::Stage::Far=>"fibre_depth<Programs>(v,front);".into(),
                    };
                    let result=match stage {
                        crate::fibre_transport::Stage::Accumulate=>"float4",
                        crate::fibre_transport::Stage::Near=>"uint2", _=>"void"};
                    let semantic=if result!="void" {":SV_Target0"} else {""};
                    let text=format!("import fibre_capture;\n{}\n[shader(\"fragment\")] {result} {entry}(Varying v,bool front:SV_IsFrontFace,uint primitive:SV_PrimitiveID,out float depth:SV_Depth) {semantic} {{v.primitive=primitive;depth=fibre_capture_depth(v);{body}}}",programs.code);
                    self.compile_entry("fibre_programs",Some(&text),(entry,"fragment"),(0,0,0),entry,None)
                        .with_context(|| format!("compile the hair programs' fibre stage {entry}"))
                })?;
                [self.fibre_vertex.clone(),fragment.clone()]
            },
            Role::ProgramCoverage {program,stage} => {
                use crate::pipelines::CoverageStage;
                let slot=self.authored_coverage.lock().unwrap().entry((program,stage)).or_default().clone();
                let (vertex,entry)=match stage {
                    CoverageStage::Depth=>(self.camera.clone(),"depth_fs"),
                    CoverageStage::Visibility=>(self.visibility[3].clone(),"masked_fs"),
                };
                [vertex,try_init(&slot,||self.program_shadow(entry,program))?.clone()]
            },

            // Position-only: no fragment stage.
            Role::OpaqueDepth => [self.opaque_depth.clone(), self.opaque_depth.clone()],
            Role::OpaqueSingleDepth => self.single_depth.clone(),
            Role::Visibility | Role::VisibilitySingle | Role::VisibilityMasked | Role::TransparentFragments => {
                let [vertex, double, single, masked_vertex, masked, fragments] = &self.visibility;
                match key.1 {
                    Role::Visibility => [vertex.clone(), double.clone()],
                    Role::VisibilitySingle => [vertex.clone(), single.clone()],
                    Role::TransparentFragments => [masked_vertex.clone(), fragments.clone()],
                    _ => [masked_vertex.clone(), masked.clone()],
                }
            }
            Role::MaskedDepth {hashed} => [self.camera.clone(), self.masked_depth[usize::from(hashed)].clone()],
            Role::Controls(features) => self.table_controls(Stage::OpaqueControls,ForwardKey::new(Stage::OpaqueControls,features).hashed()),
            Role::TransparentControls(features) => self.table_controls(Stage::Controls,ForwardKey::new(Stage::Controls,features).hashed()),
            Role::ProgramControls {program,features,transparent} => self.program_controls(if transparent {Stage::Controls} else {Stage::OpaqueControls},features,program)?,
        })
    }
    fn camera_generic(&self, kernel: crate::pipelines::ProgramKernel) -> Option<&'static native::Compiled> {
        use crate::pipelines::ProgramKernel;
        // Coverage must use the installed material specialization, including
        // imported alpha and program output proofs. The opaque generic is not
        // a coverage fallback; an authored-table preflight cannot prove this.
        // Shipped evaluators link no shadow connection through transmitting
        // surfaces: they would shadow opaque receivers as if the glass were solid.
        if self.scene.surface_coverage || self.transport_programs() || self.scene.connection_transmission || matches!(kernel,ProgramKernel::Decoupled {boundaries,..} if boundaries>0) {return None;}
        let ProgramKernel::Decoupled {stage,features,..}=kernel else {return None;};
        if !stage.table() {return None;}
        self.camera_shipped(stage,features & crate::forward_variant::FEATURE_RECEIVER_DIFFUSION!=0)
    }
    fn program_kernel(&self, kernel: crate::pipelines::ProgramKernel) -> anyhow::Result<(wgpu::ShaderModule, &'static str)> {
        Ok(self.kernel_module(kernel, false)?.built())
    }
    fn kernel_stored(&self, kernel: crate::pipelines::ProgramKernel) -> anyhow::Result<bool> {
        Ok(!matches!(self.kernel_module(kernel, true)?, Lookup::Absent))
    }
    fn constants(&self, key: GeometryKey) -> Vec<(&'static str, f64)> {
        match key.1 {
            Role::Fibre(_) | Role::ProgramCoverage {..} => Vec::new(),

            Role::OpaqueDepth
            | Role::OpaqueSingleDepth
            | Role::Visibility
            | Role::VisibilitySingle
            | Role::VisibilityMasked
            | Role::TransparentFragments
            | Role::MaskedDepth {..}
            | Role::ProgramControls {..}
            | Role::Controls(_)
            | Role::TransparentControls(_) => Vec::new(),
        }
    }
}

impl SlangForward {
    /// A ray-hit or camera kernel's module, or with `probe` whether it needs no
    /// compile (shipped, or already in the store).
    fn kernel_module(&self, kernel: crate::pipelines::ProgramKernel, probe: bool) -> anyhow::Result<Lookup<(wgpu::ShaderModule, &'static str)>> {
        use crate::pipelines::ProgramKernel;
        let shipped = |shader: &'static native::Compiled| Ok(if probe {Lookup::Stored} else {Lookup::Built((native::shader_module(&self.device,shader),shader.entry))});
        if let Some((shader,capabilities))=self.surface_fixed(kernel) {
            return Ok(if probe {Lookup::Stored} else {Lookup::Built((native::surface_module(&self.device,shader,&capabilities)?,shader.entry))});
        }
        if let ProgramKernel::Decoupled {..}=kernel {
            if let Some(shader)=self.camera_fixed(kernel) {return shipped(shader);}
            return self.camera_specialized(kernel, probe);
        }
        let (native, module, entry, kernel_call) = match kernel {
            ProgramKernel::Path { adaptive: true, .. } => (Some(&native::PATH_PATH_MAIN), "path", "path_programs_main", "path_pixel<true,Key,"),
            ProgramKernel::Path { adaptive: false, .. } => (Some(&native::PATH_PATH_FIXED), "path", "path_programs_fixed", "path_pixel<false,Key,"),
            ProgramKernel::Gi { .. } => (Some(&native::GI_RAY_COMPUTE), "gi_ray", "gi_programs", "gi_pixel<Key,"),
            ProgramKernel::Reflections { .. } => (Some(&native::REFLECTIONS_HITS_COMPUTE), "reflections_hits", "reflections_programs", "reflections_pixel<Key,"),
            ProgramKernel::Probes {directional,..} => if directional {
                (Some(&native::PROBE_UPDATE_DIRECTIONAL_TRACE),"probe_update","probe_programs","probe_directional_trace<Key,")
            } else {(Some(&native::PROBE_UPDATE_TRACE),"probe_update","probe_programs","probe_trace<Key,")},
            ProgramKernel::Decoupled {..} => unreachable!("camera kernel handled above"),
        };
        let kernel_call=if let ProgramKernel::Path {random_walk,..}=kernel {
            kernel_call.replace("Key,",&format!("{random_walk},Key,"))
        } else {kernel_call.to_owned()};
        // Programs or named attributes (VORK_ATTRIBUTES) need a per-scene compile;
        // every other kernel but the probe update (precompiled with VORK_PROBES)
        // is specialized to the scene's features.
        if matches!(kernel, ProgramKernel::Probes {features:0,..}) && self.scene.programs.is_none() && self.scene.medium.is_none() && self.scene.attributes == 0 {
            return shipped(native.unwrap());
        }
        let (program, code) = self.scene.programs.as_deref().map_or(("NoPrograms", ""), |p| ("Programs", p.code.as_str()));
        // The registered kernel's threadgroup, on both native targets.
        let threads = native.map_or((8,8,1),|n|n.threads);
        let (parameters, arguments) = match kernel {
            ProgramKernel::Probes { .. } => ("uint3 group:SV_GroupID,uint thread:SV_GroupIndex", "group,thread"),
            _ => ("uint3 gid:SV_DispatchThreadID", "gid"),
        };
        let type_arguments = match kernel {
            ProgramKernel::Path { boundaries, catchers, .. } => format!("{program},{boundaries},{catchers}"),
            _ => program.to_string(),
        };
        let text = format!(
            "// Generated by slang_forward.rs: {module} with the scene's programs and attributes at ray hits.\n\
             import vork_constants;\nimport {module};\nimport surface;\nimport frame;\n{code}\
             [shader(\"compute\")]\n[numthreads({},{},{})]\n\
             void {entry}({parameters}) {{{kernel_call}{type_arguments}>({arguments});}}\n",
            threads.0, threads.1, threads.2
        );
        let specialization = match kernel {
            ProgramKernel::Path { features, emitter_profiles, .. } => Some((features, emitter_profiles, false)),
            ProgramKernel::Reflections { features } | ProgramKernel::Gi { features } | ProgramKernel::Probes {features,..} => Some((features, false, false)),

            _ => None,
        };
        let text = if let Some((features, emitter_profiles, ray_shadows)) = specialization {
            crate::forward_variant::variant_module(features, ray_shadows, false, emitter_profiles, None, &text)
        } else { text };
        let compiled = self.entry_module(entry, Some(&text), (entry, "compute"), threads, entry, Some(kernel), None, probe)
            .with_context(|| format!("compile the {module} kernel ({entry}) with the scene's {} material programs",
                self.scene.programs.as_ref().map_or(0,|p|p.camera.len())))?;
        Ok(compiled.map(|compiled| (compiled, entry)))
    }
}

/// A module request's result: built, or (probing) whether the store holds it.
pub(crate) enum Lookup<T> { Absent, Stored, Built(T) }
impl<T> Lookup<T> {
    fn built(self) -> T {
        match self { Self::Built(value) => value, _ => unreachable!("a compile request builds") }
    }
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Lookup<U> {
        match self { Self::Built(value) => Lookup::Built(f(value)), Self::Stored => Lookup::Stored, Self::Absent => Lookup::Absent }
    }
}

/// Outlining bounds Slang's optimization graph; the device compiler still needs
/// freedom to inline large closure returns rather than spill them at each hit.
/// SPIR-V Function Control's DontInline bit is a performance hint, not semantics.
pub(crate) fn allow_driver_inlining(code: &mut [u8]) -> anyhow::Result<()> {
    anyhow::ensure!(code.len() >= 20 && code.len() % 4 == 0, "malformed SPIR-V from Slang");
    anyhow::ensure!(u32::from_le_bytes(code[..4].try_into().unwrap()) == 0x07230203, "invalid SPIR-V magic from Slang");
    let mut offset = 20;
    while offset < code.len() {
        let instruction = u32::from_le_bytes(code[offset..offset + 4].try_into().unwrap());
        let words = (instruction >> 16) as usize;
        anyhow::ensure!(words > 0 && words <= (code.len() - offset) / 4, "malformed SPIR-V instruction from Slang");
        if instruction & 0xffff == 54 { // OpFunction: result type, result ID, control, function type.
            anyhow::ensure!(words == 5, "malformed SPIR-V function from Slang");
            let control = &mut code[offset + 12..offset + 16];
            let flags = u32::from_le_bytes(control.try_into().unwrap());
            control.copy_from_slice(&(flags & !0x2).to_le_bytes());
        }
        offset += words * 4;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn shipped_surface_warm_keys_follow_executables() -> anyhow::Result<()> {
        use super::*;
        use crate::pipelines::ProgramKernel;
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        if r.adapter_info.backend!=wgpu::Backend::Vulkan {return Ok(());}
        let base=SlangScene {probes:true,no_shadow_containment:true,..Default::default()};
        let a=SlangForward::new(&r,true,false,base.clone());
        let mut changed=base.clone();changed.denoise=true;changed.no_shadow_containment=true;
        changed.camera=Some(Default::default());
        let b=SlangForward::new(&r,true,false,changed);
        let kernel=ProgramKernel::Gi {features:3};
        assert!(a.shared_kernel_key(kernel).is_some());
        assert_eq!(a.shared_kernel_key(kernel),b.shared_kernel_key(kernel));
        assert_ne!(a.shared_kernel_key(kernel),a.shared_kernel_key(ProgramKernel::Reflections {features:3}));
        assert_ne!(a.shared_kernel_key(kernel),a.shared_kernel_key(ProgramKernel::Gi {features:crate::forward_variant::FEATURE_SPECIALIZED}));
        let mut ordinary=base.clone();ordinary.textures=8;ordinary.samplers=4;
        let ordinary=SlangForward::new(&r,true,false,ordinary);
        assert_eq!(a.shared_kernel_key(kernel),ordinary.shared_kernel_key(kernel),"ordinary descriptor counts share setup's capacity");
        let mut textured=base.clone();textured.textures=vork_slang::registry::MATERIAL_TEXTURE_CAPACITY+1;
        let textures=SlangForward::new(&r,true,false,textured);
        assert_ne!(a.shared_kernel_key(kernel),textures.shared_kernel_key(kernel));
        let mut thin=base.clone();thin.connection_transmission=true;thin.ray_refraction=true;
        let thin=SlangForward::new(&r,true,false,thin);
        assert_ne!(a.shared_kernel_key(kernel),thin.shared_kernel_key(kernel),"transmitting connections require the full hit reader");
        assert_eq!(a.surface_fixed(kernel).unwrap().0.name,"surface_gi_0_1_0_0");
        assert_eq!(thin.surface_fixed(kernel).unwrap().0.name,"surface_gi_0_1_1_0");
        let mut unshadowed=base.clone();unshadowed.connection_transmission=true;
        let unshadowed=SlangForward::new(&r,false,false,unshadowed);
        assert!(unshadowed.shared_kernel_key(kernel).is_none(),"compact shipped hit lighting requires the no-transmission proof independently of primary ray bindings");
        let transport=ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::Evaluate,
            features:3,subsurface:false,emitter_profiles:false,boundaries:1};
        assert!(thin.shared_kernel_key(transport).is_some(),"ordinary single-boundary transport is shipped");
        let mut unprobed=base.clone();unprobed.ray_refraction=true;unprobed.connection_transmission=true;unprobed.probes=false;
        let unprobed=SlangForward::new(&r,true,false,unprobed);
        assert!(unprobed.shared_kernel_key(transport).is_none(),"non-probe refraction retains its validated runtime executable");

        let scene=vork_scene::v2::from_test_value(serde_json::json!({"version":2,"nodes":[]}))?;
        let instance=programs::Instance {name:"thin-shadow-test".into(),file:None,
            source:Some("import vork; struct Inputs {}; Closure surface(ShadingContext ctx,Inputs p) {Closure c={};c.transmission_weight=1.;c.geometry_thin_walled=true;return c;}".into()),inputs:Default::default(),images:Vec::new()};
        let compiled=programs::install(&r,&scene,std::iter::once(("thin",&instance)),std::path::Path::new("."))?.unwrap();
        let mut authored=base.clone();authored.connection_transmission=true;authored.programs=compiled.native;
        let authored=SlangForward::new(&r,true,false,authored);
        assert!(authored.shared_kernel_key(ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::EvaluateOpaque,features:3,subsurface:false,emitter_profiles:false,boundaries:0}).is_none(),"authored transmitting shadows must keep their dispatcher");
        assert!(!authored.scene.programs.as_ref().unwrap().coverage,"constant coverage does not remove transmission");
        assert!(authored.table_programs(Some(transport)).is_some(),"table receivers must evaluate transmitting programs even at constant coverage");
        let mut contained=base.clone();contained.no_shadow_containment=false;
        let contained=SlangForward::new(&r,true,false,contained);
        assert!(contained.shared_kernel_key(kernel).is_none(),"shipped surfaces require the no-containment proof independently of candidate layout");
        let mut transmitting=base;transmitting.connection_transmission=true;transmitting.no_shadow_containment=false;
        let c=SlangForward::new(&r,true,false,transmitting);
        assert!(c.shared_kernel_key(kernel).is_none(),"transmission must retain its different executable");
        let camera=ProgramKernel::Decoupled {stage:crate::pipelines::ShadeStage::EvaluateOpaque,
            features:3,subsurface:false,emitter_profiles:false,boundaries:0};
        assert!(a.camera_generic(camera).is_some());
        assert!(c.camera_generic(camera).is_none(),"opaque receivers still need transmitting shadow connections");
        r.errors.check()
    }

    use super::*;

    #[test]
    fn driver_inlining_clears_only_dont_inline() {
        let words = [0x07230203u32, 0x00010500, 0, 32, 0,
            (5 << 16) | 54, 1, 2, 0xe, 3, (1 << 16) | 56,
            (3 << 16) | 71, 2, 42]; // Preserve NoContraction decorations.
        let mut code: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        allow_driver_inlining(&mut code).unwrap();
        let mut expected = words;
        expected[8] = 0xc; // Preserve Pure and Const; remove only DontInline.
        assert_eq!(code, expected.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>());
        for tail in [vec![0u32], vec![(5 << 16) | 54, 1]] {
            let mut bad: Vec<u8> = words[..5].iter().copied().chain(tail).flat_map(u32::to_le_bytes).collect();
            assert!(allow_driver_inlining(&mut bad).is_err());
        }
    }

    #[test]
    fn camera_profile_follows_installed_transport() -> anyhow::Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        let kernel=crate::pipelines::ProgramKernel::Decoupled {
            stage:crate::pipelines::ShadeStage::Classify, features:0,
            subsurface:false, emitter_profiles:false, boundaries:0,
        };
        for rays in [false,true] {
            let shader=SlangForward::new(&r,rays,false,SlangScene::default());
            assert_eq!(shader.camera_fixed(kernel).unwrap().name,
                format!("camera_0_{}_classify",u8::from(rays)));
        }
        Ok(())
    }

    #[test]
    fn concurrent_layout_requests_share_one_fragment_module() -> anyhow::Result<()> {
        let Some(r) = crate::test_renderer()? else {
            return Ok(());
        };
        let shader = SlangForward::new(&r, false, false, SlangScene::default());
        let key = ForwardKey::new(Stage::OpaqueControls, 0);
        let barrier = std::sync::Barrier::new(4);
        let modules = std::thread::scope(|scope| {
            (0..4)
                .map(|_| scope.spawn(|| {
                    barrier.wait();
                    shader.generic(key.stage,false)
                }))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(modules.iter().all(|module| module == &modules[0]));
        assert_eq!(shader.generic(key.stage,false), modules[0]);
        Ok(())
    }
}
