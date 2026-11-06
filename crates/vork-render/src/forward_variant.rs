//! The forward variant key and its generated entry module. build.rs includes
//! this file too, so the precompiled generic and runtime variants come from one
//! definition (docs/design/shaders.md: a variant is one `IVariant` type).

/// CPU-only installed capabilities. Shaders receive folded bools, not this mask.
/// Keep independently authored response and transport flags beyond the first word.
pub type FeatureMask = u128;
/// Coverage samples of multisampled raster passes (constants::RASTER_MSAA_SAMPLES).
pub const RASTER_MSAA_SAMPLES: u32 = 4;
pub const SHADING_LAYERS: u8 = 1;
pub const SHADING_METAL: u8 = 2;
// Install-time capability key: layer/metal bits, one texture-presence bit per input,
// then independent coat/fuzz, F82 tint, coverage and scene light-mix bits.
pub const FEATURE_COAT: FeatureMask = 1 << 44;
pub const FEATURE_FUZZ: FeatureMask = 1 << 45;
pub const FEATURE_TINT: FeatureMask = 1 << 46;
pub const FEATURE_MASK: FeatureMask = 1 << 47;
pub const FEATURE_SIDED: FeatureMask = 1 << 48;
pub const FEATURE_PUNCTUAL: FeatureMask = 1 << 49;
pub const FEATURE_FINITE: FeatureMask = 1 << 50;
/// Installed finite-emitter geometry. Radius/angle animation may enable an ellipse;
/// the authored area shape itself never changes after install.
pub const FEATURE_RECTANGLE: FeatureMask = 1 << 57;
pub const FEATURE_ELLIPSE: FeatureMask = 1 << 58;
/// Installed shadow-ray consumers, independent of energy and other TLAS uses
/// such as refraction or thickness. Generic variants conservatively retain rays.
pub const FEATURE_SHADOW_RAYS: FeatureMask = 1 << 60;
/// A Blend program: the draw shades each lobe of a mixed surface.
pub const FEATURE_LOBES: FeatureMask = 1 << 55;
/// A program with a lit stage: the draw shades the closure it returns.
pub const FEATURE_LIT: FeatureMask = 1 << 56;
/// Installed transmission and possible solid boundaries, conservative for programs/tracks.
pub const FEATURE_TRANSMISSION: FeatureMask = 1 << 61;
pub const FEATURE_SOLID_TRANSMISSION: FeatureMask = 1 << 62;
pub const FEATURE_ANISOTROPY: FeatureMask = 1 << 64;
pub const FEATURE_FILM: FeatureMask = 1 << 65;
/// Camera results consumed by installed screen effects; shared by all material bins.
pub const FEATURE_CAMERA_RECEIVERS: FeatureMask = 1 << 66;
pub const FEATURE_REFLECTIONS: FeatureMask = 1 << 67;
pub const FEATURE_INDIRECT_CONTROLS: FeatureMask = 1 << 68;
/// Only the combined SSS/screen-refraction composition kernel captures a second receiver.
pub const FEATURE_RECEIVER_DIFFUSION: FeatureMask = 1 << 69;
pub const FEATURE_SPECIALIZED: FeatureMask = 1 << 63;
/// Stored evaluation records' optional words. `camera_config.words` sizes each record
/// from the scene's mask, so a kernel that writes evaluations must agree on these exactly:
/// one that wrote more would overwrite the next record's words.
pub const FEATURE_RECORD_WORDS: FeatureMask = FEATURE_CAMERA_RECEIVERS | FEATURE_REFLECTIONS | FEATURE_INDIRECT_CONTROLS;
/// An unspecialized key takes `FEATURE_RECORD_WORDS` from its mask instead of enabling them.
pub const FEATURE_EXACT_RECORD: FeatureMask = 1 << 70;
/// Hashed (dithered) camera coverage: the entry declares SV_PrimitiveID.
/// Declaring that input costs every draw of the pipeline, measured +8% of a
/// motion-blurred car's controls pass on L4, so table entries declare it only
/// for dithered materials (shipped generics come with and without it).
/// Pipeline roles add it (never Material::features); it selects only the entry
/// signature, not the `Key : IVariant` capabilities.
pub const FEATURE_DITHERED: FeatureMask = 1 << 69;
/// Four emitter kinds, matching Light.position.w (sun, point, spot, area).
pub const FEATURE_LIGHT_SHIFT: u32 = 51;
pub const FEATURE_LIGHT_TYPES: FeatureMask = 15 << FEATURE_LIGHT_SHIFT;
/// Texture-presence bits from bit 2: every OpenPBR input plus glTF occlusion.
pub const TEXTURE_FEATURES: usize = 38;
const _: () = assert!(TEXTURE_FEATURES + 2 <= 44);

/// The control-pass fragment stage a variant compiles: the opaque or the
/// nearest transparent control targets (colour is the decoupled shade's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Controls,
    OpaqueControls,
}
impl Stage {
    /// The fragment stages, each with a precompiled generic, indexed by discriminant
    /// (SlangForward::generic).
    pub const ALL: [Self; 2] = [Self::Controls, Self::OpaqueControls];
}
const _: () = {
    let mut i = 0;
    while i < Stage::ALL.len() {
        assert!(Stage::ALL[i] as usize == i);
        i += 1;
    }
};

/// The compile-time capabilities of one control-pass pipeline: a materials.rs
/// feature word. `true` is always correct: every capability's code is gated on
/// its material weight as well, so a key with more capabilities renders the
/// same pixels. Controls light nothing, so no shadow technique specializes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ForwardKey {
    pub stage: Stage,
    pub features: FeatureMask,
}

impl ForwardKey {
    pub const fn new(stage: Stage, features: FeatureMask) -> Self {Self {stage, features}}
    /// The precompiled generic (the superset fallback) for a stage, with or
    /// without hashed coverage.
    pub const fn generic(stage: Stage, hashed: bool) -> Self {
        Self::new(stage, (SHADING_LAYERS | SHADING_METAL) as FeatureMask | if hashed {FEATURE_DITHERED} else {0})
    }
    pub fn is_generic(self) -> bool {self == Self::generic(self.stage, self.hashed())}
    /// The entry declares SV_PrimitiveID for hashed coverage (raster_accepts).
    pub const fn hashed(self) -> bool {self.features & FEATURE_DITHERED != 0}
    /// The entry point, (entry, stage). Its vertex stage is the precompiled
    /// `depth.main_vs`, shared with the depth prepass.
    pub fn entry(self) -> (&'static str, &'static str) {
        let entry = match self.stage {
            Stage::Controls => "transparent_controls_fs",
            Stage::OpaqueControls => "controls_fs",
        };
        (entry, "fragment")
    }
    /// The module name, also its file stem.
    pub fn name(self) -> String {
        let stage = match self.stage {
            Stage::Controls => "controls",
            Stage::OpaqueControls => "opaque_controls",
        };
        if self.is_generic() {
            return format!("{stage}_generic{}", if self.hashed() {"_hashed"} else {""});
        }
        format!("{stage}_{:016x}", self.features)
    }
    /// The generated composition: one conforming type and the stage entry. An
    /// unspecialized word keeps every capability, as the WGSL overrides' defaults do.
    /// `programs` is a scene's `struct Programs : IProgram` and its imports (Slang),
    /// applied in place of NoPrograms.
    pub fn module_with(self, programs: Option<&str>) -> String {
        let program = if programs.is_some() { "Programs" } else { "NoPrograms" };
        let (entry, _) = self.entry();
        let function = match self.stage {Stage::Controls => "transparent_controls", Stage::OpaqueControls => "controls"};
        // Hashed coverage needs the primitive, and so may a program's
        // ctx.primitive_id. Opaque and cutout table keys never read it.
        let (primitive, assign) = if self.hashed() || programs.is_some() {
            (", uint primitive : SV_PrimitiveID", "v.primitive=primitive;")
        } else {
            ("", "")
        };
        let body = format!(
            "import forward;\n[shader(\"fragment\")]\n\
             Output {entry}(Varying v, bool front : SV_IsFrontFace{primitive}) {{{assign}return {function}<Key,{program}>(v,front);}}\n"
        );
        variant_module(self.features, false, false, false, programs, &body)
    }
}

/// Values shared by literal Slang keys and late SPIR-V specialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability { Bool(bool), Uint(u32) }
impl Capability {
    fn ty(self) -> &'static str { match self { Self::Bool(_)=>"bool", Self::Uint(_)=>"uint" } }
    fn literal(self) -> String { match self { Self::Bool(value)=>value.to_string(), Self::Uint(value)=>format!("{value}u") } }
}

// Field order is the internal SPIR-V SpecId ABI. Both the shipped declarations
// and per-scene values come from this list, including texture masks and light kinds.
pub fn variant_capabilities(f: FeatureMask, ray_shadows: bool, subsurface: bool, emitter_profiles: bool) -> Vec<(&'static str, Capability)> {
    let specialized = f & FEATURE_SPECIALIZED != 0;
    let bit = |b: FeatureMask| !specialized || f & b != 0;
    let record = |b: FeatureMask| if f & FEATURE_EXACT_RECORD != 0 {f & b != 0} else {bit(b)};
    let textures = (f >> 2) & ((1 << TEXTURE_FEATURES) - 1);
    let (lo, hi, light_types) = if specialized {
        (
            textures as u32,
            (textures >> 32) as u32,
            ((f & FEATURE_LIGHT_TYPES) >> FEATURE_LIGHT_SHIFT) as u32,
        )
    } else {
        (u32::MAX, u32::MAX, 15)
    };
    vec![
        ("layered", Capability::Bool(f & SHADING_LAYERS as FeatureMask != 0)),
        ("metal", Capability::Bool(f & SHADING_METAL as FeatureMask != 0)),
        ("ray_shadows", Capability::Bool(ray_shadows)),
        ("subsurface", Capability::Bool(subsurface)),
        ("specialized", Capability::Bool(specialized)),
        ("textures_lo", Capability::Uint(lo)),
        ("textures_hi", Capability::Uint(hi)),
        ("has_coat", Capability::Bool(bit(FEATURE_COAT))),
        ("has_fuzz", Capability::Bool(bit(FEATURE_FUZZ))),
        ("has_anisotropy", Capability::Bool(bit(FEATURE_ANISOTROPY))),
        ("has_film", Capability::Bool(bit(FEATURE_FILM))),
        ("has_tint", Capability::Bool(bit(FEATURE_TINT))),
        ("has_mask", Capability::Bool(bit(FEATURE_MASK))),
        ("has_sided", Capability::Bool(bit(FEATURE_SIDED))),
        ("has_punctual", Capability::Bool(bit(FEATURE_PUNCTUAL))),
        ("has_finite", Capability::Bool(bit(FEATURE_FINITE))),
        ("has_rectangle", Capability::Bool(bit(FEATURE_RECTANGLE))),
        ("has_ellipse", Capability::Bool(bit(FEATURE_ELLIPSE))),
        ("has_shadow_rays", Capability::Bool(bit(FEATURE_SHADOW_RAYS))),
        ("has_transmission", Capability::Bool(bit(FEATURE_TRANSMISSION))),
        ("has_solid_transmission", Capability::Bool(bit(FEATURE_SOLID_TRANSMISSION))),
        ("has_emitter_profiles", Capability::Bool(emitter_profiles)),
        ("has_camera_receivers", Capability::Bool(record(FEATURE_CAMERA_RECEIVERS))),
        ("has_reflections", Capability::Bool(record(FEATURE_REFLECTIONS))),
        ("has_indirect_controls", Capability::Bool(record(FEATURE_INDIRECT_CONTROLS))),
        ("has_lobes", Capability::Bool(bit(FEATURE_LOBES))),
        ("has_lit", Capability::Bool(bit(FEATURE_LIT))),
        ("light_types", Capability::Uint(light_types)),
    ]
}

pub fn variant_module(f: FeatureMask, ray_shadows: bool, subsurface: bool, emitter_profiles: bool, programs: Option<&str>, body: &str) -> String {
    use std::fmt::Write;
    let mut text=format!("// Generated by forward_variant.rs: features {f:#x}.\nimport variant;\nimport stream;\nimport surface;\n{}\nstruct Key : IVariant {{\n",programs.unwrap_or_default());
    for (name,value) in variant_capabilities(f,ray_shadows,subsurface,emitter_profiles) {
        writeln!(text,"    static const {} {name} = {};",value.ty(),value.literal()).unwrap();
    }
    write!(text,"}};\n{body}").unwrap();
    text
}

/// Compile the capability graph once; the Vulkan driver folds scene values at
/// pipeline creation. No material feature combination requires another Slang artifact.
#[allow(dead_code)] // Used by build_slang.rs and interface tests.
pub fn late_variant_module(body: &str, ray_shadows: bool) -> String {
    use std::fmt::Write;
    let fields=variant_capabilities(0,false,false,false);
    let mut text="import variant;\nimport stream;\nimport surface;\n".to_owned();
    for (id,(name,value)) in fields.iter().enumerate() {
        if *name=="ray_shadows" || *name=="subsurface" && !ray_shadows {continue;}
        let zero=match value {Capability::Bool(_)=>"false",Capability::Uint(_)=>"0u"};
        writeln!(text,"[vk::constant_id({id})] const {} vork_cap_{name} = {zero};",value.ty()).unwrap();
    }
    text.push_str("struct Key : IVariant {\n");
    for (name,value) in fields {
        // This capability selects descriptor types, so it belongs to the
        // resource interface rather than the driver's late feature values.
        if name=="ray_shadows" {
            writeln!(text,"    static const bool ray_shadows = {ray_shadows};").unwrap();
            continue;
        }
        if name=="subsurface" && !ray_shadows {
            text.push_str("    static const bool subsurface = false;\n");
            continue;
        }
        writeln!(text,"    static const {} {name} = vork_cap_{name};",value.ty()).unwrap();
    }
    write!(text,"}};\n{body}").unwrap();
    text
}

/// slangc arguments shared by the build and runtime variant compiles.
pub const SLANG_FLAGS: &[&str] = &["-matrix-layout-row-major"];
/// Native speed profile, shared by build-time kernels and runtime path compilation.
pub const NATIVE_OPTIMIZATION: &str = "-O2";
/// Per backend: (file extension, slangc target, target flags).
pub const SPIRV_TARGET: (&str, &str, &[&str]) = (
    "spv",
    "spirv",
    &["-emit-spirv-directly", "-fvk-use-entrypoint-name"],
);
pub fn material_registers(material: u32) -> Vec<String> {
    crate::binding_abi::material_registers(material)
}

/// The irradiance volume's three consecutive Metal buffer registers.
pub fn probe_registers(first: u32) -> [String; 3] {
    std::array::from_fn(|offset| format!("-DVORK_PROBE_{}_REGISTER={}{}", ["IRRADIANCE", "VISIBILITY", "GRID"][offset], if offset==2 {"u"}else{"t"}, first + offset as u32))
}

/// Compute hit-kernel tables start after that kernel's group-0 buffers.
/// Shared by embedded compilation and scene specializations; native layout tests
/// verify every index against the renderer's actual bind-group entries.
pub fn hit_kernel_registers(module: &str, adaptive: bool, probes: bool) -> Vec<String> {
    let material = match module {
        "gi_ray" => 10,
        "reflections_hits" => 13,
        "probe_update" => 13,
        "path" => 15 + u32::from(adaptive),
        _ => return Vec::new(),
    };
    let mut defines = material_registers(material);
    defines.push("-DVORK_HIT_KERNEL".to_owned());
    defines.push(format!("-DVORK_HIT_REGISTER=b{}",material+5+if probes {3} else {0}));
    if module == "gi_ray" {defines.push(format!("-DVORK_GI_MEDIUM_REGISTER=u{}",material+6+if probes {3} else {0}));}
    if module == "reflections_hits" {
        defines.push(format!("-DVORK_REFLECTION_FIELDS_REGISTER=b{}",material+6+if probes {3} else {0}));
        defines.push(format!("-DVORK_REFLECTION_MEDIUM_REGISTER=b{}",material+7+if probes {3} else {0}));
    }
    if module == "path" {
        defines.push("-DVORK_CATCHER_REGISTER=u13".to_owned());
        defines.push("-DVORK_CATCHER_PIXELS_REGISTER=u14".to_owned());
    }
    if module == "path" && adaptive {defines.push("-DVORK_PATH_ADAPTIVE_NATIVE".to_owned());}
    if probes {
        defines.extend(probe_registers(material + 5));
    }
    defines
}

// Native callbacks need link-time visibility, not externally callable library
// exports. Slang keeps `export` roots alive with unspecialized mutable parameters,
// which Metal cannot emit (shader-slang/slang#11981). C linkage lets unused
// callbacks disappear and reachable callbacks specialize from the shader entry.
// The token macros leave strings/comments and authored program analysis alone.
// MSL carries no `#line` paths: Metal's system cache keys libraries by source
// text, so build or workspace paths would make identical shaders miss it.
pub const METAL_TARGET: (&str, &str, &[&str]) = ("metal", "metal", &["-line-directive-mode", "none", "-DVORK_METAL", "-Dexport=[__unsafeForceInlineEarly] __extern_cpp", "-Dextern=__extern_cpp extern"]);

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn late_capabilities_cover_the_shader_interface() {
        let source=include_str!("../shaders/variant.slang");
        let interface=source.split_once("interface IVariant {").unwrap().1.split_once("};").unwrap().0;
        let names:std::collections::BTreeSet<_>=interface.lines().filter_map(|line| {
            let line=line.trim().strip_prefix("static const ")?;
            Some(line.split_whitespace().nth(1).unwrap().trim_end_matches(';'))
        }).collect();
        let fields=variant_capabilities(FEATURE_SPECIALIZED,false,false,false);
        assert_eq!(names,fields.iter().map(|(name,_)|*name).collect());
        assert_eq!(names.len(),fields.len(),"every capability has one SpecId");
        let module=late_variant_module("",false);
        for (id,(name,_)) in fields.iter().enumerate() {
            if *name=="ray_shadows" {assert!(module.contains("ray_shadows = false;"));continue;}
            if *name=="subsurface" {assert!(module.contains("subsurface = false;"));continue;}
            assert!(module.contains(&format!("[vk::constant_id({id})]")));
            assert!(module.contains(&format!("{name} = vork_cap_{name};")));
        }
    }
    /// Only the hashed generic declares SV_PrimitiveID among table keys; program
    /// keys keep it for ctx.primitive_id. native.rs checks the compiled generics.
    #[test]
    fn only_hashed_coverage_declares_the_primitive() {
        let declares=|key:ForwardKey,programs:Option<&str>| {
            let module=key.module_with(programs);
            let entry=module.lines().find(|l|l.contains(key.entry().0)).expect("entry line").to_owned();
            assert_eq!(entry.contains("SV_PrimitiveID"),entry.contains("v.primitive=primitive;"),"{entry}");
            entry.contains("SV_PrimitiveID")
        };
        for stage in Stage::ALL {
            for hashed in [false,true] {
                let key=ForwardKey::generic(stage,hashed);
                assert_eq!(declares(key,None),hashed,"{stage:?}");
            }
            assert!(declares(ForwardKey::new(stage,FEATURE_SPECIALIZED|FEATURE_MASK),Some("")),"programs may read ctx.primitive_id");
            assert!(ForwardKey::generic(stage,false).is_generic() && ForwardKey::generic(stage,true).is_generic());
            assert_ne!(ForwardKey::generic(stage,false).name(),ForwardKey::generic(stage,true).name());
        }
    }

    #[test]
    fn response_capabilities_do_not_alias_transparent_transport() {
        let response=FEATURE_ANISOTROPY|FEATURE_FILM;
        let transport=FEATURE_TRANSMISSION|FEATURE_SOLID_TRANSMISSION;
        assert_eq!(response&transport,0);
        for (flags,aniso,film,trans,solid) in [
            (FEATURE_ANISOTROPY,true,false,false,false),
            (FEATURE_FILM,false,true,false,false),
            (FEATURE_TRANSMISSION,false,false,true,false),
            (transport,false,false,true,true),
            (response|transport,true,true,true,true),
        ] {
            let module=variant_module(FEATURE_SPECIALIZED|flags,false,false,false,None,"");
            for (name,enabled) in [("has_anisotropy",aniso),("has_film",film),("has_transmission",trans),("has_solid_transmission",solid)] {
                assert!(module.contains(&format!("static const bool {name} = {enabled};")),"{name}, flags {flags:x}");
            }
        }
    }
}
