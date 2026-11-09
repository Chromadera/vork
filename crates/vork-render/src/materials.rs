//! Tagged material records and installed textures (GPU mips) and samplers.
use super::*;
use ::vork_scene::v2::{OPENPBR, OpenPbrKind, OpenPbrParam};

// Fatal errors, transmission warning, evaluation demand and current-view capacity feedback.
pub(crate) const SHADER_DIAGNOSTICS_INIT: [u32; vork_slang::registry::SHADER_DIAGNOSTICS_WORDS as usize] = [0, u32::MAX, 0, 0, 0, 0];
pub(crate) const SHADER_DIAGNOSTICS_BYTES: u64 = std::mem::size_of_val(&SHADER_DIAGNOSTICS_INIT) as u64;

/// One material: OpenPBR inputs as constants, each optionally multiplied by a channel
/// of a texture tap. glTF materials map onto the same inputs at import.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MaterialRecord {
    /// x program id (0 = std/openpbr); y first tap word in the word buffer; z flags
    /// (bit 1 double-sided, bit 2 vertex alpha multiplies opacity); w first program input word.
    pub(crate) tag: [u32; 4],
    /// Reserved, coverage integration (1 = continuous, 2 = sampled, 3 = binary), glTF occlusion strength, disable transparent shadows.
    pub(crate) extra: [f32; 4],
    /// One u32 per OpenPBR input, then glTF occlusion: low 28 bits address
    /// the tap (0 = none); high bits encode the channel and normal mode.
    pub(crate) inputs: [u32; constants::MATERIAL_INPUT_WORDS],
    /// Each input's constant (a texture's factor), in OpenPBR table order; normals and
    /// tangents reserve XYZ, with scalar map scales in their first word.
    pub(crate) values: [f32; VALUE_WORDS],
}
impl MaterialRecord {
    fn get(&self, name: &str) -> &[f32] {
        let (index, p) = param(name);
        let word = value_word(index);
        &self.values[word..word + value_width(p.kind)]
    }
    fn bound(&self, name: &str) -> bool {
        let input = param(name).0;
        self.inputs[input] & constants::MATERIAL_TAP_INDEX_MASK != 0
    }
}
/// Constant words per input: three for colours/vectors, one otherwise.
const fn value_width(kind: OpenPbrKind) -> usize {
    match kind {
        OpenPbrKind::Color3 | OpenPbrKind::Vector3 => 3,
        _ => 1,
    }
}
const VALUE_COUNT: usize = {
    let mut words = 0;
    let mut i = 0;
    while i < OPENPBR.len() {
        words += value_width(OPENPBR[i].kind);
        i += 1;
    }
    words
};
/// The constants padded to whole vec4s for WGSL.
pub(crate) const VALUE_WORDS: usize = VALUE_COUNT.next_multiple_of(4);
/// The tap byte after the OpenPBR inputs: glTF occlusion (indirect visibility).
pub(crate) const OCCLUSION_INPUT: usize = OPENPBR.len();
const _: () = assert!(OCCLUSION_INPUT < constants::MATERIAL_INPUT_WORDS * 4);
/// First constant word of OpenPBR parameter `index`.
pub(crate) fn value_word(index: usize) -> usize {
    OPENPBR[..index].iter().map(|p| value_width(p.kind)).sum()
}
/// Channel codes in an input's tap byte.
const CHANNEL_RGB: u8 = 4;
/// Record flags (MaterialRecord::tag z).
use constants::{FLAG_DOUBLE_SIDED, FLAG_VERTEX_ALPHA};
fn param(name: &str) -> (usize, &'static OpenPbrParam) {
    let index = OPENPBR
        .iter()
        .position(|p| p.name == name)
        .expect("an OpenPBR parameter");
    (index, &OPENPBR[index])
}

/// A texture tap: one table texture with its sampler, read through an affine UV map
/// of TEXCOORD_0 or TEXCOORD_1 (glTF KHR_texture_transform order: scale, rotate, offset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tap {
    pub word: u64,
    pub uv: u32,
    pub scale: [f32; 2],
    pub rotation: f32,
    pub offset: [f32; 2],
}
impl Tap {
    fn plain(word: u64) -> Self {
        Self {
            word,
            uv: 0,
            scale: [1.; 2],
            rotation: 0.,
            offset: [0.; 2],
        }
    }
    /// Words in the material word buffer: table word, flags (bit 0 TEXCOORD_1, bit 1 the
    /// map turns UV axes so tangent-space normals take derivative tangents), then a
    /// column-major 2x2 matrix and the offset.
    fn words(&self) -> [u32; constants::MATERIAL_TAP_WORDS] {
        let (s, c) = self.rotation.sin_cos();
        // Positive repeats and offsets preserve the authored tangent axes.
        // Rotation, reflection, collapsed axes or a different UV set need the
        // transformed derivatives instead.
        let basis = self.uv != 0 || self.scale.iter().any(|s| *s <= 0.) || self.rotation != 0.;
        [
            self.word as u32,
            self.uv | u32::from(basis) * constants::FLAG_UV_BASIS,
            (c * self.scale[0]).to_bits(),
            (s * self.scale[0]).to_bits(),
            (-s * self.scale[1]).to_bits(),
            (c * self.scale[1]).to_bits(),
            self.offset[0].to_bits(),
            self.offset[1].to_bits(),
            (self.word >> 32) as u32,
        ]
    }
}
/// A record with its taps, before the word buffer places them.
#[derive(Clone)]
pub(crate) struct RecordBuilder {
    pub(crate) record: MaterialRecord,
    pub(crate) taps: Vec<[u32; constants::MATERIAL_TAP_WORDS]>,
}
impl RecordBuilder {
    /// Every input at its OpenPBR default; single-sided, opaque, no taps.
    pub(crate) fn openpbr() -> Self {
        let mut record = MaterialRecord::zeroed();
        for (index, p) in OPENPBR.iter().enumerate() {
            let word = value_word(index);
            match p.kind {
                OpenPbrKind::Vector3 => record.values[word] = 1.,
                _ => record.values[word..word + p.default.len()].copy_from_slice(p.default),
            }
        }
        Self {
            record,
            taps: Vec::new(),
        }
    }
    pub(crate) fn set(&mut self, name: &str, values: &[f32]) {
        let (index, p) = param(name);
        let word = value_word(index);
        if p.kind == OpenPbrKind::Vector3 && values.len() == 3 {
            let marker = 1u32 << 31;
            if self.record.bound(name) && values[0] == values[1] && values[2] == 1. {
                // Preserve the scalar normal-map path when XYZ is just XY scale.
                self.record.inputs[index] &= !marker;
                self.record.values[word..word + 3].copy_from_slice(&[values[0], 0., 0.]);
                return;
            }
            if !self.record.bound(name) && values.iter().all(|v| *v == 0.) {
                self.record.inputs[index] &= !marker;
            } else {
                self.record.inputs[index] |= marker;
            }
        }
        debug_assert!(values.len() == value_width(p.kind) || (p.kind == OpenPbrKind::Vector3 && values.len() == 1));
        self.record.values[word..word + values.len()].copy_from_slice(values);
    }
    pub(crate) fn get(&self, name: &str) -> &[f32] {
        self.record.get(name)
    }
    /// Multiply input `input` (an OpenPBR index, or OCCLUSION_INPUT) by `channel` of `tap`.
    fn bind_index(&mut self, input: usize, tap: Tap, channel: u8) -> Result<()> {
        let words = tap.words();
        let index = match self.taps.iter().position(|t| *t == words) {
            Some(index) => index,
            None => {
                self.taps.push(words);
                self.taps.len() - 1
            }
        };
        self.record.inputs[input] = (index as u32 + 1) | u32::from(channel) << 28;
        Ok(())
    }
    pub(crate) fn bind(&mut self, name: &str, tap: Tap, channel: u8) -> Result<()> {
        self.bind_index(param(name).0, tap, channel)
    }
    pub(crate) fn bound(&self, name: &str) -> bool {
        self.record.bound(name)
    }
}
/// glTF material extensions mapped onto OpenPBR inputs here (assets may require them).
/// Read through `extension_value`: enabling a typed gltf feature for one of these would
/// empty that lookup.
pub(crate) const GLTF_OPENPBR_EXTENSIONS: &[&str] = &[
    "KHR_materials_unlit",
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_specular",
];
/// The installed scene material table. Frames whose material inputs animate take a
/// private copy of the record storage ([`MaterialTable::frame_with_images`]); the rest share `bind`.
pub(crate) struct MaterialTable {
    shared: Option<FrameMaterials>,
    records: Vec<u8>,
    views: Vec<TextureSlot>,
    samplers: Vec<wgpu::Sampler>,
    layout: wgpu::BindGroupLayout,
}
/// One frame slot's material records and the group 1 binding that reads them. Frame
/// values write animated inputs at [`value_location`] through the one change tracker,
/// whose history is per buffer, so each slot's copy receives what it lacks.
pub(crate) struct FrameMaterials {
    pub(crate) shader_errors: wgpu::Buffer,
    pub(crate) records: wgpu::Buffer,
    pub(crate) bind: wgpu::BindGroup,
}
impl MaterialTable {
    pub(crate) fn bind(&self) -> &wgpu::BindGroup {
        &self
            .shared
            .as_ref()
            .expect("render-image table needs frame views")
            .bind
    }
    /// Material records then program/medium input words, as bound in group 1.
    pub(crate) fn records(&self) -> &wgpu::Buffer {
        &self
            .shared
            .as_ref()
            .expect("render-image table needs frame views")
            .records
    }
    pub(crate) fn has_render_images(&self) -> bool {
        self.views
            .iter()
            .any(|slot| matches!(slot, TextureSlot::Render(_)))
    }
    pub(crate) fn frame_with_images(
        &self,
        r: &Renderer,
        images: &[wgpu::TextureView],
    ) -> FrameMaterials {
        let views: Vec<_> = self
            .views
            .iter()
            .map(|slot| match slot {
                TextureSlot::File(view) => view.clone(),
                TextureSlot::Render(index) => images[*index].clone(),
            })
            .collect();
        frame_materials(r, &self.layout, &self.records, &views, &self.samplers)
    }
}
fn frame_materials(
    r: &Renderer,
    layout: &wgpu::BindGroupLayout,
    bytes: &[u8],
    views: &[wgpu::TextureView],
    samplers: &[wgpu::Sampler],
) -> FrameMaterials {
    let records = init(
        &r.device,
        "material records",
        bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    let shader_errors = init(&r.device, "shader diagnostics", bytemuck::cast_slice(&SHADER_DIAGNOSTICS_INIT), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
    let mut views: Vec<_> = views.iter().collect();
    let mut samplers: Vec<_> = samplers.iter().collect();
    views.resize(views.len().max(vork_slang::registry::MATERIAL_TEXTURE_CAPACITY as usize),views[0]);
    samplers.resize(samplers.len().max(vork_slang::registry::MATERIAL_SAMPLER_CAPACITY as usize),samplers[0]);
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material table"),
        layout,
        entries: &[
            entry(4, shader_errors.as_entire_binding()),
            entry(0, records.as_entire_binding()),
            entry(3, records.as_entire_binding()),
            entry(1, wgpu::BindingResource::TextureViewArray(&views)),
            entry(2, wgpu::BindingResource::SamplerArray(&samplers)),
        ],
    });
    FrameMaterials { records, bind, shader_errors }
}
// The install-time capability key is defined beside the Slang variant it selects.
pub(crate) use crate::forward_variant::{
    FeatureMask, FEATURE_ANISOTROPY, FEATURE_FILM, FEATURE_COAT, FEATURE_ELLIPSE, FEATURE_FINITE, FEATURE_FUZZ, FEATURE_RECTANGLE, FEATURE_LIGHT_SHIFT, FEATURE_LIGHT_TYPES,
    FEATURE_SHADOW_RAYS, FEATURE_TRANSMISSION, FEATURE_SOLID_TRANSMISSION, FEATURE_LIT, FEATURE_LOBES, FEATURE_MASK, FEATURE_PUNCTUAL, FEATURE_SIDED, FEATURE_SPECIALIZED, FEATURE_TINT,
    SHADING_LAYERS, SHADING_METAL,
};
const _: () = assert!(OCCLUSION_INPUT + 1 == crate::forward_variant::TEXTURE_FEATURES);
const _: () = assert!(OCCLUSION_INPUT + 2 < 44);

/// Bound drawn (layout, capability) permutations before requesting pipelines.
/// Excess keys share their conservative union. If the mandatory layouts alone
/// exceed the budget, use generic shading for those drawn roles instead.
pub(crate) fn bound_features(
    materials: &mut [Material],
    drawn: impl Iterator<Item = (crate::geometry::VertexLayout, usize)>,
) {
    let drawn: std::collections::HashSet<_> = drawn.collect();
    let keys: std::collections::HashSet<_> = drawn
        .iter()
        .map(|&(layout, i)| (layout, materials[i].features))
        .collect();
    if keys.len() > constants::SCENE_SHADING_VARIANTS {
        let layouts: std::collections::HashSet<_> =
            drawn.iter().map(|&(layout, _)| layout).collect();
        let mut union = keys.into_iter().fold(0, |a, (_, b)| a | b);
        if layouts.len() > constants::SCENE_SHADING_VARIANTS {
            // Use the superset itself: no extra background variant for these layouts.
            union = FeatureMask::from(SHADING_LAYERS | SHADING_METAL);
        }
        for (_, i) in drawn {
            materials[i].features = union;
        }
    }
}
/// One OpenPBR input's texture capability in the installed shading key.
pub(crate) fn texture_feature(input: &str) -> FeatureMask {
    1 << (param(input).0 + 2)
}
pub(crate) struct Material {
    /// See `PendingMaterial::zero_opacity`.
    pub(crate) zero_opacity: bool,
    pub(crate) program: u32,
    /// Source-domain emission may be nonzero in a sample: builtin emission or
    /// the shared surface closure evaluation.
    pub(crate) source_emission: bool,
    pub(crate) hair: bool,
    /// May select physical fibre scattering; diffuse-only Hair is excluded.
    pub(crate) physical_hair: bool,
    pub(crate) features: FeatureMask,
    pub(crate) transparent: bool,
    /// Any possible transmission (thin or solid), independent of alpha coverage.
    pub(crate) transmissive: bool,
    /// A closure can require a solid screen-receiver query. Thin sheets only sample colour.
    pub(crate) solid_transmission: bool,
    /// May absorb over a straight occupied light connection.
    pub(crate) solid_absorption: bool,
    /// May produce a thin sheet, including program or animated closures.
    pub(crate) thin_transmission: bool,
    #[cfg(test)]
    pub(crate) shadow_coverage: bool,
    /// Cutout or blended surface coverage, independently of transparent shadows.
    pub(crate) surface_coverage: bool,
    /// Material coverage can change without an object transform.
    pub(crate) shadow_animated: bool,
    pub(crate) alpha_mask: bool,
    /// Coverage surface method: camera coverage is hashed per triangle, so its
    /// raster entries read SV_PrimitiveID (forward_variant::FEATURE_DITHERED).
    pub(crate) dithered: bool,
    pub(crate) double_sided: bool,
}

// Texture installation precedes the single scene-wide material-record upload.
#[derive(Clone)]
pub(crate) struct PendingMaterial {
    pub(crate) hair: bool,
    /// May select physical fibre scattering; diffuse-only Hair is excluded.
    pub(crate) physical_hair: bool,
    pub(crate) record: MaterialRecord,
    /// Source glTF accessor sets feeding the two packed UV channels.
    pub(crate) uv_sources: [u32; 2],
    emission_animated: bool,
    pub(crate) source_emission: bool,
    /// Texture taps; install appends them to the word buffer and points tag y at them.
    pub(crate) taps: Vec<[u32; constants::MATERIAL_TAP_WORDS]>,
    alpha_mask: bool,
    double_sided: bool,
    transparent: bool,
    opacity_animated: bool,
    // Reserves shader features even when their initial weights are zero.
    shading: u8,
    response_animated: FeatureMask,
    /// Possible program responses, replaced by compiler proofs at installation.
    pub(crate) response_program: FeatureMask,
    // An animated material's name: its record must not merge with an identical static one.
    unique: Option<String>,
    // Reserves the transmission path: a nonzero weight or a track on it.
    pub(crate) transmissive: bool,
    /// A Blend program: its draws shade each lobe (FEATURE_LOBES).
    pub(crate) lobes: bool,
    /// A program with a lit stage (FEATURE_LIT).
    pub(crate) lit: bool,
}
// Records hold table indices, so record bytes identify a material.
pub(crate) type MaterialKey = Vec<u8>;
impl PendingMaterial {
    /// Static unbound zero coverage needs no draw. Vertex-alpha draws retain
    /// ordinary coverage when their sampling policy could vary.
    fn zero_opacity(&self) -> bool {
        self.record.tag[0] == 0 && !self.opacity_animated
            && self.record.tag[2] & constants::FLAG_VERTEX_ALPHA == 0
            && !self.record.bound("geometry_opacity") && self.record.get("geometry_opacity") == [0.]
            && !self.record.bound("alpha_cutoff") && self.record.get("alpha_cutoff") != [0.]
    }

    /// A material program's record: OpenPBR defaults with the authored coverage.
    pub(crate) fn program(id: u32, double_sided: bool) -> Self {
        let mut record = RecordBuilder::openpbr().record;
        record.tag[0] = id;
        let mut material = Self {
            record,
            uv_sources: [0, 1],
            emission_animated: false,
            source_emission: false,
            taps: Vec::new(),
            alpha_mask: false,
            double_sided: false,
            transparent: false,
            opacity_animated: false,
            shading: 0,
            response_animated: 0,
            response_program: if id != 0 {FEATURE_ANISOTROPY|FEATURE_FILM|FEATURE_COAT|FEATURE_FUZZ|FEATURE_TINT} else {0},
            unique: None,
            transmissive: false,
            lobes: false,
            lit: false,
            hair: false,
            physical_hair: false,
        };
        material.coverage(double_sided);
        material
    }
    /// A medium-only material retains its closed boundary without a visible shell.
    pub(crate) fn medium_boundary() -> Self {
        let mut material = Self::program(0, false);
        let mut builder = RecordBuilder::openpbr();
        builder.set("geometry_opacity", &[0.]);
        material.record = builder.record;
        material.record.tag[2] |= vork_slang::registry::FLAG_MEDIUM_BOUNDARY;
        material.integrate_opacity(Some(vork_scene::Opacity::Transmittance), false, false);
        material
    }
    /// Programs can vary opacity/transmission per point. Reserve their passes
    /// at install; the evaluated closure determines the contribution per sample.
    pub(crate) fn integrate_opacity(&mut self, method: Option<vork_scene::Opacity>, transmission: bool, animated: bool) {
        self.transmissive |= transmission;
        self.transparent = self.transmissive || method==Some(vork_scene::Opacity::Transmittance);
        let method=method.unwrap_or(if self.transmissive {vork_scene::Opacity::Transmittance} else {vork_scene::Opacity::Coverage});
        self.alpha_mask |= method == vork_scene::Opacity::Coverage && !self.transmissive;
        self.record.extra[1] = match method { vork_scene::Opacity::Transmittance => 1., vork_scene::Opacity::Coverage => 2. };
        self.opacity_animated |= animated;
    }
    /// Authored sidedness.
    fn coverage(&mut self, double_sided: bool) {
        self.double_sided = double_sided;
        self.record.tag[2] =
            (self.record.tag[2] & !FLAG_DOUBLE_SIDED) | u32::from(double_sided) * FLAG_DOUBLE_SIDED;
    }
    /// Bit n: a tap reads TEXCOORD_n.
    pub(crate) fn uv_sets(&self) -> u8 {
        self.taps
            .iter()
            .filter(|tap| tap[1] & constants::MATERIAL_ATTRIBUTE_FLAG == 0)
            .fold(0, |sets, tap| {
                sets | 1 << (tap[1] & constants::FLAG_UV_SECOND)
            })
    }
    /// UV-only ray candidates cannot supply these bound attribute taps.
    pub(crate) fn attribute_coverage(&self) -> bool {
        self.record.tag[0] == 0 && ["geometry_opacity", "alpha_cutoff"].iter().any(|name| {
            let tap = self.record.inputs[param(name).0] & constants::MATERIAL_TAP_INDEX_MASK;
            tap != 0 && self.taps[tap as usize - 1][1] & constants::MATERIAL_ATTRIBUTE_FLAG != 0
        })
    }
    pub(crate) fn surface_coverage(&self) -> bool {
        self.record.extra[1] > 0.
    }
    pub(crate) fn key(&self) -> MaterialKey {
        let mut key = bytemuck::bytes_of(&self.record).to_vec();
        key.extend_from_slice(bytemuck::cast_slice(&self.taps));
        key.extend_from_slice(bytemuck::cast_slice(&self.uv_sources));
        if let Some(name) = &self.unique {
            key.extend_from_slice(name.as_bytes());
        }
        key
    }

    /// Shared by pre-pipeline technique selection and the final scene plan.
    /// Programs, tracks and texture-driven wall type remain conservative.
    pub(crate) fn solid_transmission(&self) -> bool {
        self.wall_transmission(false)
    }
    pub(crate) fn solid_absorption(&self) -> bool {
        self.solid_transmission() && (self.record.tag[0]!=0 || self.unique.is_some()
            || (self.record.get("transmission_depth")[0]>0. || self.record.bound("transmission_depth"))
                && (self.record.get("transmission_color").iter().any(|&v|v<1.) || self.record.bound("transmission_color")))
    }
    fn wall_transmission(&self,thin: bool) -> bool {
        let input=param("geometry_thin_walled").0;
        self.transmissive && (self.record.tag[0]!=0 || self.unique.is_some()
            || (self.record.values[value_word(input)]!=0.)==thin
            || self.record.inputs[input]&constants::MATERIAL_TAP_INDEX_MASK!=0)
    }
    fn emits(&self) -> bool {
        if self.record.tag[0]!=0 {self.source_emission} else {
            self.emission_animated || (self.record.get("emission_luminance")[0]>0. && self.record.get("emission_color")!=[0.;3])
        }
    }
    pub(crate) fn physical_emitter(&self) -> bool {self.emits() && !self.transparent && !self.zero_opacity()}

    pub(crate) fn finish(self) -> Material {
        let value = |name| self.record.values[value_word(param(name).0)];
        let mut features = FEATURE_SPECIALIZED;
        if self.transmissive { features |= FEATURE_TRANSMISSION; }
        if self.solid_transmission() { features |= FEATURE_SOLID_TRANSMISSION; }
        for input in 0..=OCCLUSION_INPUT {
            if self.record.inputs[input] & (constants::MATERIAL_TAP_INDEX_MASK | (1u32 << 31)) != 0 {
                features |= 1 << (input + 2);
            }
        }
        // Tracks retain both layers; programs use their compiled response proofs.
        let dynamic_layers = self.shading & SHADING_LAYERS != 0;
        if dynamic_layers || self.response_program & FEATURE_COAT != 0 || value("coat_weight") > 0. {
            features |= FEATURE_COAT;
        }
        if dynamic_layers || self.response_program & FEATURE_FUZZ != 0 || value("fuzz_weight") > 0. {
            features |= FEATURE_FUZZ;
        }
        // Any animated material may animate specular_color. Keep F82 in that case.
        if self.unique.is_some()
            || self.response_program & FEATURE_TINT != 0
            || self.record.values[value_word(param("specular_color").0)..][..3] != [1.; 3]
            || self.record.inputs[param("specular_color").0]
                & constants::MATERIAL_TAP_INDEX_MASK
                != 0
        {
            features |= FEATURE_TINT;
        }
        // Hair programs return a separate typed closure, not arbitrary OpenPBR
        // fields. Their Chiang roughness/coat and diffuse fallback do not use
        // the ordinary surface anisotropy or thin-film response.
        let programmed_response = if self.hair {0} else {self.response_program};
        for (input, capability) in [("specular_roughness_anisotropy", FEATURE_ANISOTROPY), ("thin_film_weight", FEATURE_FILM)] {
            let i = param(input).0;
            if (self.response_animated | programmed_response) & capability != 0 || value(input) > 0.
                || self.record.inputs[i] & constants::MATERIAL_TAP_INDEX_MASK != 0 {
                features |= capability;
            }
        }
        if self.alpha_mask {
            features |= FEATURE_MASK;
        }
        if !self.double_sided {
            features |= FEATURE_SIDED;
        }
        if self.lobes {
            features |= FEATURE_LOBES;
        }
        if self.lit {
            features |= FEATURE_LIT;
        }
        let shading = self.shading
            | if self.record.tag[0] != 0 {
                SHADING_LAYERS | SHADING_METAL
            } else {
                0
            }
            | if value("coat_weight") > 0. || value("fuzz_weight") > 0. {
                SHADING_LAYERS
            } else {
                0
            }
            | if value("base_metalness") > 0. {
                SHADING_METAL
            } else {
                0
            };
        let source_emission=self.emits();
        Material {
            zero_opacity: self.zero_opacity(),
            program:self.record.tag[0],
            source_emission,
            hair: self.hair,
            physical_hair: self.physical_hair,
            features: features | FeatureMask::from(shading),
            #[cfg(test)]
            shadow_coverage: self.surface_coverage(),
            surface_coverage: self.surface_coverage(),
            shadow_animated: self.surface_coverage() && self.opacity_animated,
            alpha_mask: self.alpha_mask,
            dithered: self.record.extra[1]==2.,
            double_sided: self.double_sided,
            transparent: self.transparent,
            transmissive: self.transmissive,
            thin_transmission: self.wall_transmission(true),
            solid_transmission: self.solid_transmission(),
            solid_absorption: self.solid_absorption(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum TextureSlot {
    File(wgpu::TextureView),
    Render(usize),
}
impl From<wgpu::TextureView> for TextureSlot {
    fn from(view: wgpu::TextureView) -> Self {
        Self::File(view)
    }
}

pub(crate) struct TextureInstaller<'a> {
    pub(crate) preparation: compressed_textures::Preparation,
    pub(crate) upload_ms: std::cell::Cell<f64>,
    pub(crate) cache_hits: std::cell::Cell<usize>,
    pub(crate) r: &'a Renderer,
    pub(crate) footage: std::collections::BTreeMap<String, crate::footage::Installed>,
    pub(crate) fps: vork_time::Time,
    pub(crate) render_images: &'a std::collections::BTreeMap<String, usize>,
    pub(crate) pipeline_ms: std::cell::Cell<f64>,
    resident:
        std::cell::RefCell<std::collections::HashMap<(blake3::Hash, bool, bool), wgpu::TextureView>>,
    /// glTF sampler codes; the last field marks spec §2 (anisotropic) program samplers.
    samplers: std::collections::HashMap<(u32, u32, Option<u32>, Option<u32>, bool), wgpu::Sampler>,
    file_views: std::collections::HashMap<(std::path::PathBuf, bool), wgpu::TextureView>,
    // The scene material table: one entry per distinct view and sampler.
    table: Table<TextureSlot>,
    sampler_table: Table<wgpu::Sampler>,
}
struct Table<T> {
    entries: Vec<T>,
    index: std::collections::HashMap<T, u32>,
}
impl<T: Clone + Eq + std::hash::Hash> Table<T> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: Default::default(),
        }
    }
    fn insert(&mut self, value: T) -> u32 {
        *self.index.entry(value.clone()).or_insert_with(|| {
            self.entries.push(value);
            self.entries.len() as u32 - 1
        })
    }

}
/// Mip generation samples the previous level (shaders/mip.slang).
pub(crate) const MIP_LAYOUT: [wgpu::BindGroupLayoutEntry; 2] = [
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    },
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    },
];
/// A full-screen pipeline writing one level of `format` from the level above.
pub(crate) fn mip_resources(r: &Renderer) -> &([wgpu::RenderPipeline; 3], wgpu::Sampler) {
    r.pipelines.mips.get_or_init(|| {
        let create = |format| mip_pipeline(r,format);
        let pipelines = std::thread::scope(|scope| {
            let linear = scope.spawn(|| create(wgpu::TextureFormat::Rgba8Unorm));
            let srgb = scope.spawn(|| create(wgpu::TextureFormat::Rgba8UnormSrgb));
            let float = create(wgpu::TextureFormat::Rgba16Float);
            [
                linear.join().expect("mip compiler panicked"),
                srgb.join().expect("mip compiler panicked"),
                float,
            ]
        });
        let sampler = r.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("trilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        (pipelines, sampler)
    })
}

/// Retained downsample steps shared by uploaded textures and published captures.
pub(crate) struct MipChain {
    pipeline: wgpu::RenderPipeline,
    steps: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
}
impl MipChain {
    pub(crate) fn new(d: &wgpu::Device, texture: &wgpu::Texture, format: Option<wgpu::TextureFormat>, pipeline: &wgpu::RenderPipeline, sampler: &wgpu::Sampler) -> Self {
        let steps=(1..texture.mip_level_count()).map(|level| {
            let view=|base_mip_level| texture.create_view(&wgpu::TextureViewDescriptor {
                format, base_mip_level, mip_level_count: Some(1), ..Default::default()
            });
            let source=view(level-1);
            let target=view(level);
            let bind=d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mip source"), layout: &pipeline.get_bind_group_layout(0),
                entries: &[entry(0,wgpu::BindingResource::TextureView(&source)),
                    entry(1,wgpu::BindingResource::Sampler(sampler))],
            });
            (bind,target)
        }).collect();
        Self {pipeline:pipeline.clone(),steps}
    }
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        for (index,(bind,target)) in self.steps.iter().enumerate() {
            let attachments=[Some(color_attachment(target,wgpu::Color::BLACK))];
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("mip"), color_attachments:&attachments, depth_stencil_attachment:None,
                timestamp_writes: timestamps.as_ref().and_then(|writes| {
                    let begin = writes.beginning_of_pass_write_index.filter(|_| index==0);
                    let end = writes.end_of_pass_write_index.filter(|_| index+1==self.steps.len());
                    (begin.is_some() || end.is_some()).then_some(wgpu::RenderPassTimestampWrites {
                        query_set: writes.query_set, beginning_of_pass_write_index: begin, end_of_pass_write_index: end,
                    })
                }),
                occlusion_query_set:None,multiview_mask:None,
            });
            pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,bind,&[]);pass.draw(0..3,0..1);
        }
    }
    pub(crate) fn is_empty(&self) -> bool { self.steps.is_empty() }
}

fn mip_pipeline(
    r: &Renderer,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let label=match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => "sRGB material mips",
        wgpu::TextureFormat::Rgba16Float => "float mips",
        _ => "linear material mips",
    };
    let vs = native::shader_module(&r.device, &native::MIP_VS);
    let fs = native::shader_module(&r.device, &native::MIP_FS);
    let group = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &MIP_LAYOUT,
        });
    let layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&group)],
            immediate_size: 0,
        });
    r.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some(native::MIP_VS.entry),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some(native::MIP_FS.entry),
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: r.cache.as_ref(),
    })
}
impl<'a> TextureInstaller<'a> {
    pub(crate) fn new(
        r: &'a Renderer,
        render_images: &'a std::collections::BTreeMap<String, usize>,
    ) -> Self {
        Self {
            r,
            render_images,
            footage: Default::default(), fps: vork_scene::v2::Render::default().fps,
            upload_ms: std::cell::Cell::new(0.),
            cache_hits: std::cell::Cell::new(0),
            pipeline_ms: std::cell::Cell::new(0.),
            preparation: Default::default(),
            resident: Default::default(),
            samplers: Default::default(),
            file_views: Default::default(),
            table: Table::new(),
            sampler_table: Table::new(),
        }
    }
    pub(crate) fn counts(&mut self) -> Result<(u32,u32)> {
        if self.table.entries.is_empty() {
            let white = self.upload(None, false, true)?;
            let sampler = self.material_sampler(None);
            self.slot(white,sampler);
        }
        Ok((u32::try_from(self.table.entries.len())?.max(vork_slang::registry::MATERIAL_TEXTURE_CAPACITY),u32::try_from(self.sampler_table.entries.len())?.max(vork_slang::registry::MATERIAL_SAMPLER_CAPACITY)))
    }
    /// Full-width texture and sampler indices, split into two GPU words.
    pub(crate) fn slot(&mut self, view: impl Into<TextureSlot>, sampler: wgpu::Sampler) -> u64 {
        u64::from(self.table.insert(view.into())) | u64::from(self.sampler_table.insert(sampler)) << 32
    }
    /// The scene material table: record storage (records, then program input words)
    /// plus the texture and sampler tables, bound once as material group 1.
    pub(crate) fn table(
        &mut self,
        layout: &wgpu::BindGroupLayout,
        records: Vec<u8>,
    ) -> Result<MaterialTable> {
        crate::gpu_memory::storage_binding(
            &self.r.device.limits(),
            "material table (records and program inputs)",
            records.len() as u64,
        )?;
        if self.table.entries.is_empty() {
            let white = self.upload(None, false, true)?;
            let sampler = self.material_sampler(None);
            self.slot(white, sampler);
        }
        let views: Vec<_> = self
            .table
            .entries.iter().cloned()
            .collect();
        let samplers: Vec<_> = self
            .sampler_table
            .entries.iter().cloned()
            .collect();
        let file_views: Option<Vec<_>> = views
            .iter()
            .map(|v| match v {
                TextureSlot::File(view) => Some(view.clone()),
                TextureSlot::Render(_) => None,
            })
            .collect();
        let shared =
            file_views.map(|views| frame_materials(self.r, layout, &records, &views, &samplers));
        Ok(MaterialTable {
            shared,
            records,
            views,
            samplers,
            layout: layout.clone(),
        })
    }
    fn mip_resources(&self) -> &([wgpu::RenderPipeline; 3], wgpu::Sampler) {
        let first = self.r.pipelines.mips.get().is_none();
        let start = Instant::now();
        let resources = mip_resources(self.r);
        if first {
            self.pipeline_ms
                .set(self.pipeline_ms.get() + start.elapsed().as_secs_f64() * 1000.);
        }
        resources
    }
    /// Upload a program image file with its v2 colour space and sampler. Channels
    /// keep their file precision (see `upload`); IES retains linear half-float strength.
    pub(crate) fn file_image(&mut self, path: PathBuf, srgb: bool) -> Result<wgpu::TextureView> {
        let memo = (path.clone(), srgb);
        if let Some(view) = self.file_views.get(&memo) {
            return Ok(view.clone());
        }
        let source =
            std::fs::read(&path).with_context(|| format!("read image {}", path.display()))?;
        let ies = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ies"));
        let mut key = blake3::Hasher::new();
        if ies {
            key.update(b"ies-cycles-linear-v1");
        }
        key.update(&source);
        key.update(path.extension().unwrap_or_default().as_encoded_bytes());
        let key = key.finalize();
        let cached_path = self.r.disk_cache.path("native-decoded-texture", 4, key);
        let data = if let Some(data) = read_decoded(&cached_path) {
            data
        } else {
            let data = decoded_data(
                crate::texture_source::decode(
                    &path,
                    &source,
                    self.r.device.limits().max_texture_dimension_2d,
                )
                .with_context(|| format!("decode image {}", path.display()))?,
            )?;
            let _ = write_decoded(&cached_path, &data);
            data
        };
        let data = Image::from(data);
        let view = if ies {
            ensure!(!srgb, "IES images require linear color space");
            self.upload_ies(&data, key)
        } else {
            self.upload(Some((&data, key)), srgb, true)
        }
        .with_context(|| format!("upload image {}", path.display()))?;
        self.file_views.insert(memo, view.clone());
        Ok(view)
    }
    pub(crate) fn scene_image(
        &mut self,
        name: &str,
        image: &::vork_scene::v2::Image,
        base: &Path,
    ) -> Result<(TextureSlot, wgpu::Sampler)> {
        let descriptor = crate::programs::Image {
            input: String::new(),
            source: crate::texture_source::TextureSource::scene(name, image),
            srgb: image.is_srgb(),
            sampler: image.sampler(),
        };
        self.program_image(&descriptor, base)
    }
    pub(crate) fn program_image(
        &mut self,
        image: &crate::programs::Image,
        base: &Path,
    ) -> Result<(TextureSlot, wgpu::Sampler)> {
        use crate::constants::PROGRAM_IMAGE_ANISOTROPY;
        use ::vork_scene::v2::{MagFilter, MinFilter, Wrap};
        let view = match &image.source {
            crate::texture_source::TextureSource::File(path) => {
                self.file_image(base.join(path), image.srgb)?.into()
            }
            crate::texture_source::TextureSource::Media {name,file,time} => {
                if !self.footage.contains_key(name) {
                    self.footage.insert(name.clone(), crate::footage::Installed::new(self.r,name,&base.join(file),time.clone(),image.srgb,self.fps)?);
                }
                self.footage[name].view.clone().into()
            }
            crate::texture_source::TextureSource::Render(name) => TextureSlot::Render(
                *self
                    .render_images
                    .get(name)
                    .with_context(|| format!("uninstalled render image {name}"))?,
            ),
        };
        // glTF sampler codes, so program and glTF samplers share one map.
        let (wrap_s, wrap_t, min, mag) = image.sampler.gltf_codes();
        let device = &self.r.device;
        let sampler = self
            .samplers
            .entry((wrap_s, wrap_t, Some(min), Some(mag), true))
            .or_insert_with(|| {
                let address = |w| match w {
                    Wrap::Repeat => wgpu::AddressMode::Repeat,
                    Wrap::ClampToEdge => wgpu::AddressMode::ClampToEdge,
                    Wrap::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
                };
                let filter = |linear| {
                    if linear {
                        wgpu::FilterMode::Linear
                    } else {
                        wgpu::FilterMode::Nearest
                    }
                };
                let s = image.sampler;
                let mag_linear = s.mag_filter == MagFilter::Linear;
                let min_linear = matches!(
                    s.min_filter,
                    MinFilter::Linear | MinFilter::LinearMipmapNearest | MinFilter::LinearMipmapLinear
                );
                let trilinear = s.min_filter == MinFilter::LinearMipmapLinear;
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("program image"),
                    address_mode_u: address(s.wrap_s),
                    address_mode_v: address(s.wrap_t),
                    mag_filter: filter(mag_linear),
                    min_filter: filter(min_linear),
                    mipmap_filter: if matches!(
                        s.min_filter,
                        MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear
                    ) {
                        wgpu::MipmapFilterMode::Linear
                    } else {
                        wgpu::MipmapFilterMode::Nearest
                    },
                    // glTF minFilter without mipmaps samples only the base level.
                    lod_max_clamp: if matches!(s.min_filter, MinFilter::Nearest | MinFilter::Linear) {
                        0.
                    } else {
                        32.
                    },
                    // Spec §2 Image: linear trilinear filtering is also anisotropic.
                    anisotropy_clamp: if trilinear && mag_linear { PROGRAM_IMAGE_ANISOTROPY } else { 1 },
                    ..Default::default()
                })
            })
            .clone();
        Ok((view, sampler))
    }
    fn material_sampler(&mut self, texture: Option<gltf::Texture<'_>>) -> wgpu::Sampler {
        use gltf::texture::WrappingMode;
        let sampler = texture.as_ref().map(|t| t.sampler());
        let key = (
            sampler
                .as_ref()
                .map_or(WrappingMode::Repeat, |s| s.wrap_s()) as u32,
            sampler
                .as_ref()
                .map_or(WrappingMode::Repeat, |s| s.wrap_t()) as u32,
            sampler
                .as_ref()
                .and_then(|s| s.min_filter())
                .map(|v| v as u32),
            sampler
                .as_ref()
                .and_then(|s| s.mag_filter())
                .map(|v| v as u32),
            false,
        );
        self.samplers
            .entry(key)
            .or_insert_with(|| material_sampler(&self.r.device, texture))
            .clone()
    }
    /// glTF channels declare whether alpha is sampled; authored images retain it.
    pub(crate) fn upload(
        &self,
        image: Option<(&Image, blake3::Hash)>,
        srgb: bool,
        preserve_alpha: bool,
    ) -> Result<wgpu::TextureView> {
        use gltf::image::Format;
        let upload_start = Instant::now();
        let pipeline_ms_before = self.pipeline_ms.get();
        // Only block compression drops unsampled alpha; half-float uploads keep
        // every channel, so either request shares one resident texture.
        let preserve_alpha = preserve_alpha || image.is_some_and(|(im, _)| !eight_bit(im.format));
        let key = (
            image.map_or_else(|| blake3::hash(b"vork-white-fallback"), |(_, hash)| hash),
            srgb,
            preserve_alpha,
        );
        let image = image.map(|(image, _)| image);
        if let Some(view) = self.resident.borrow().get(&key) {
            self.cache_hits.set(self.cache_hits.get() + 1);
            self.upload_ms
                .set(self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.);
            return Ok(view.clone());
        }
        // 16-bit integer and float channels upload uncompressed as half float;
        // 8-bit block compression and the 8-bit fallback would quantize them.
        if let Some(im) = image.filter(|im| !eight_bit(im.format)) {
            let view = self.upload_half(im.data()?, srgb)?;
            self.resident.borrow_mut().insert(key, view.clone());
            self.upload_ms.set(
                self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.
                    - (self.pipeline_ms.get() - pipeline_ms_before),
            );
            return Ok(view);
        }
        if let Some(im) = image {
            if let Some(view) =
                compressed_textures::load_or_encode(
                    self.r, &self.preparation, key.0, srgb, preserve_alpha, im.width, im.height, None,
                )?
            {
                self.upload_ms
                    .set(self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.);
                self.resident.borrow_mut().insert(key, view.clone());
                return Ok(view);
            }
        }
        let (w, h, mut pixels) = if let Some(im) = image {
            let im = im.data()?;
            let channels = match im.format {
                Format::R8 => 1,
                Format::R8G8 => 2,
                Format::R8G8B8 => 3,
                Format::R8G8B8A8 => 4,
                _ => bail!("8-bit upload received {:?} channels", im.format),
            };
            let pixels = im.pixels.as_slice();
            let mut rgba = Vec::with_capacity(im.width as usize * im.height as usize * 4);
            for p in pixels.chunks_exact(channels) {
                rgba.extend_from_slice(&match channels {
                    1 => [p[0], p[0], p[0], 255],
                    2 => [p[0], p[0], p[0], p[1]],
                    3 => [p[0], p[1], p[2], 255],
                    _ => [p[0], p[1], p[2], p[3]],
                });
            }
            (im.width, im.height, rgba)
        } else {
            (1, 1, vec![255; 4])
        };

        if image.is_some() {
            match compressed_textures::load_or_encode(
                self.r, &self.preparation, key.0, srgb, preserve_alpha, w, h, Some(&mut pixels),
            ) {
                Ok(Some(view)) => {
                    self.upload_ms
                        .set(self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.);
                    self.resident.borrow_mut().insert(key, view.clone());
                    return Ok(view);
                }
                Err(error) => eprintln!("texture compression unavailable: {error:#}"),
                _ => {}
            }
        }
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let view = self.uncompressed_texture("material texture", w, h, format, &pixels, true);
        self.upload_ms.set(
            self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.
                - (self.pipeline_ms.get() - pipeline_ms_before),
        );
        self.resident.borrow_mut().insert(key, view.clone());
        Ok(view)
    }
    /// IES angular strength: one linear half-float level (no mips), with any
    /// intensity beyond half-float range rejected rather than clamped.
    fn upload_ies(&self, image: &Image, hash: blake3::Hash) -> Result<wgpu::TextureView> {
        let key = (hash, false, true);
        if let Some(view) = self.resident.borrow().get(&key) {
            self.cache_hits.set(self.cache_hits.get() + 1);
            return Ok(view.clone());
        }
        let upload_start = Instant::now();
        let im = image.data()?;
        ensure!(
            im.pixels.chunks_exact(4).all(|v| half::f16::from_f32(f32::from_ne_bytes([v[0], v[1], v[2], v[3]])).is_finite()),
            "IES intensity exceeds half-float texture range"
        );
        let texels = half_texels(im, false)?;
        let view = self.uncompressed_texture("IES angular image", im.width, im.height,
            wgpu::TextureFormat::Rgba16Float, bytemuck::cast_slice(&texels), false);
        self.resident.borrow_mut().insert(key, view.clone());
        self.upload_ms.set(self.upload_ms.get() + upload_start.elapsed().as_secs_f64() * 1000.);
        Ok(view)
    }
    /// 16-bit integer or float channels as RGBA16F with a full float mip chain,
    /// the format of environment, IES and render images (and Blender's default
    /// half-float GPU images). sRGB decodes to linear here: no 16-bit sRGB format exists.
    fn upload_half(&self, im: &gltf::image::Data, srgb: bool) -> Result<wgpu::TextureView> {
        let texels = half_texels(im, srgb)?;
        Ok(self.uncompressed_texture("half-float material texture", im.width, im.height,
            wgpu::TextureFormat::Rgba16Float, bytemuck::cast_slice(&texels), true))
    }
    /// An RGBA texture from tightly packed texels, with a full mip chain from
    /// mip.slang when `mips` (formats of `mip_resources`).
    fn uncompressed_texture(&self, label: &str, w: u32, h: u32, format: wgpu::TextureFormat, texels: &[u8], mips: bool) -> wgpu::TextureView {
        let levels = if mips { 32 - w.max(h).leading_zeros() } else { 1 };
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        if levels > 1 {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let texture = crate::gpu_memory::texture(&self.r.device, &wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(w, h),
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        self.r.queue.write_texture(
            texture.as_image_copy(),
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * format.block_copy_size(None).unwrap()),
                rows_per_image: Some(h),
            },
            extent(w, h),
        );
        if levels > 1 {
            let (pipelines, sampler) = self.mip_resources();
            let mut encoder = self.r.device.create_command_encoder(&Default::default());
            let pipeline = match format {
                wgpu::TextureFormat::Rgba8Unorm => &pipelines[0],
                wgpu::TextureFormat::Rgba8UnormSrgb => &pipelines[1],
                _ => &pipelines[2],
            };
            MipChain::new(&self.r.device, &texture, None, pipeline, sampler).encode(&mut encoder, None);
            crate::command_finish::Segments::submit(&self.r.queue, encoder);
        }
        texture.create_view(&Default::default())
    }
}

pub(crate) fn eight_bit(format: gltf::image::Format) -> bool {
    use gltf::image::Format::*;
    matches!(format, R8 | R8G8 | R8G8B8 | R8G8B8A8)
}

/// Linear RGBA half-float texels from 16-bit integer or 32-bit float channels,
/// expanded like 8-bit images (grey to RGB, missing alpha opaque). Values above
/// 1 and below 0 survive; magnitudes beyond half-float range clamp to +-65504,
/// as for environment radiance. Non-finite texels are an error.
pub(crate) fn half_texels(im: &gltf::image::Data, srgb: bool) -> Result<Vec<u16>> {
    use gltf::image::Format::*;
    let (channels, size) = match im.format {
        R16 => (1, 2),
        R16G16 => (2, 2),
        R16G16B16 => (3, 2),
        R16G16B16A16 => (4, 2),
        R32G32B32FLOAT => (3, 4),
        R32G32B32A32FLOAT => (4, 4),
        format => bail!("unsupported image channels {format:?}"),
    };
    // image::DynamicImage exposes u16 and f32 samples in native endian.
    let sample = |v: &[u8]| {
        if size == 2 {
            f32::from(u16::from_ne_bytes([v[0], v[1]])) / 65535.
        } else {
            f32::from_ne_bytes([v[0], v[1], v[2], v[3]])
        }
    };
    let max = crate::constants::HALF_FLOAT_MAX;
    let mut texels = Vec::with_capacity(im.width as usize * im.height as usize * 4);
    for p in im.pixels.chunks_exact(channels * size) {
        let c = |i: usize| sample(&p[i * size..]);
        let rgba = match channels {
            1 => [c(0), c(0), c(0), 1.],
            2 => [c(0), c(0), c(0), c(1)],
            3 => [c(0), c(1), c(2), 1.],
            _ => [c(0), c(1), c(2), c(3)],
        };
        for (i, mut v) in rgba.into_iter().enumerate() {
            ensure!(v.is_finite(), "image texels must be finite");
            if srgb && i < 3 {
                v = crate::texture_source::decode_srgb(v);
            }
            texels.push(half::f16::from_f32(v.clamp(-max, max)).to_bits());
        }
    }
    Ok(texels)
}

/// Source UV indices are accessor names, not GPU channel indices. Resolve only
/// the sets the material reads; unused UV layers need no packed vertex storage.
#[derive(Default)]
struct GltfUvSources([Option<u32>; 2]);
impl GltfUvSources {
    fn bind(&mut self, source: u32) -> Result<u32> {
        let slot = if let Some(slot) = self.0.iter().position(|v| *v == Some(source)) {
            slot
        } else {
            let preferred = (source < 2).then_some(source as usize)
                .filter(|slot| self.0[*slot].is_none());
            let slot = preferred.or_else(|| self.0.iter().position(Option::is_none))
                .context("glTF material reads more than two distinct UV sets")?;
            self.0[slot] = Some(source);
            slot
        };
        // Authored glTF tangents describe source UV0. Every other source needs
        // derivative tangents, even when installed into packed channel zero.
        Ok(slot as u32 | if source != 0 { constants::FLAG_UV_BASIS } else { 0 })
    }
    fn sources(&self) -> [u32; 2] {
        [self.0[0].unwrap_or(0), self.0[1].unwrap_or(1)]
    }
}

/// A glTF textureInfo's coordinates: its set and KHR_texture_transform (scale, rotate, offset).
fn gltf_uv(
    tex_coord: u32,
    transform: Option<&serde_json::Value>,
) -> Result<(u32, [f32; 2], f32, [f32; 2])> {
    let pair = |key: &str, default: [f32; 2]| -> Result<[f32; 2]> {
        transform
            .and_then(|t| t.get(key))
            .map_or(Ok(default), |v| Ok(serde_json::from_value(v.clone())?))
    };
    let set = transform
        .and_then(|t| t.get("texCoord")?.as_u64())
        .map_or(tex_coord, |v| v as u32);
    let rotation = transform
        .and_then(|t| t.get("rotation")?.as_f64())
        .unwrap_or(0.) as f32;
    let (scale, offset) = (pair("scale", [1.; 2])?, pair("offset", [0.; 2])?);
    ensure!(
        scale.iter().chain(&offset).all(|v| v.is_finite()) && rotation.is_finite(),
        "texture transform must be finite"
    );
    Ok((set, scale, rotation, offset))
}

/// glTF material import: one mapping of glTF 2.0 and its KHR material extensions onto
/// OpenPBR inputs (OpenPBR's glTF mapping), sharing the record and taps of authored materials.
pub(crate) fn install_gltf(
    t: &mut TextureInstaller,
    doc: &gltf::Document,
    mat: Option<&gltf::Material>,
    images: &[Image],
    keys: &[blake3::Hash],
) -> Result<PendingMaterial> {
    use gltf::material::AlphaMode;
    let mut b = RecordBuilder::openpbr();
    // glTF's own defaults where it has them (OpenPBR's differ): white base, metal 1,
    // roughness 1, IOR 1.5, emission off, double-sided for primitives without a material.
    let pbr = mat.map(|m| m.pbr_metallic_roughness());
    let base = pbr.as_ref().map_or([1.; 4], |p| p.base_color_factor());
    let unlit = mat.is_some_and(|m| m.extension_value("KHR_materials_unlit").is_some());
    let alpha_mode = mat.map_or(AlphaMode::Opaque, |m| m.alpha_mode());
    b.set("base_color", &base[..3]);
    b.set(
        "base_metalness",
        &[pbr.as_ref().map_or(1., |p| p.metallic_factor())],
    );
    b.set(
        "specular_roughness",
        &[pbr.as_ref().map_or(1., |p| p.roughness_factor())],
    );
    b.set("specular_ior", &[mat.and_then(|m| m.ior()).unwrap_or(1.5)]);
    b.set(
        "emission_color",
        &mat.map_or([0.; 3], |m| m.emissive_factor()),
    );
    b.set(
        "emission_luminance",
        &[mat.and_then(|m| m.emissive_strength()).unwrap_or(1.)],
    );
    // Opaque glTF ignores alpha; otherwise base colour alpha (and vertex alpha) is opacity.
    let alpha = alpha_mode != AlphaMode::Opaque;
    b.set("geometry_opacity", &[if alpha { base[3] } else { 1. }]);
    let mut uv_sources = GltfUvSources::default();
    let mut tap = |texture: gltf::Texture<'_>,
                   tex_coord: u32,
                   transform: Option<&serde_json::Value>,
                   srgb: bool,
                   preserve_alpha: bool|
     -> Result<Tap> {
        let source = texture.source().index();
        let view = t.upload(
            Some((&images[source], keys[source])), srgb, preserve_alpha,
        )?;
        let sampler = t.material_sampler(Some(texture));
        let (source_uv, scale, rotation, offset) = gltf_uv(tex_coord, transform)?;
        let uv = uv_sources.bind(source_uv)?;
        Ok(Tap {
            word: t.slot(view, sampler),
            uv,
            scale,
            rotation,
            offset,
        })
    };
    // This extension is typed by the gltf crate, so it is absent from
    // extension_value (unlike normal/occlusion textureInfo extensions).
    let info_transform = |info: &gltf::texture::Info<'_>| {
        info.texture_transform().map(|transform| {
            let mut value = serde_json::json!({"offset":transform.offset(),
                "scale":transform.scale(),"rotation":transform.rotation()});
            if let Some(uv) = transform.tex_coord() { value["texCoord"] = uv.into(); }
            value
        })
    };
    if let Some(info) = pbr.as_ref().and_then(|p| p.base_color_texture()) {
        let base_tap = tap(
            info.texture(),
            info.tex_coord(),
            info_transform(&info).as_ref(),
            true,
            alpha,
        )?;
        b.bind("base_color", base_tap, CHANNEL_RGB)?;
        if alpha {
            b.bind("geometry_opacity", base_tap, 3)?;
        }
    }
    let mut extra = [0.; 4];
    let mut weight = 0.;
    if !unlit {
        if let Some(info) = pbr.as_ref().and_then(|p| p.metallic_roughness_texture()) {
            let mr = tap(
                info.texture(),
                info.tex_coord(),
                info_transform(&info).as_ref(),
                false,
                false,
            )?;
            b.bind("specular_roughness", mr, 1)?;
            b.bind("base_metalness", mr, 2)?;
        }
        if let Some(info) = mat.and_then(|m| m.emissive_texture()) {
            let emission = tap(
                info.texture(),
                info.tex_coord(),
                info_transform(&info).as_ref(),
                true,
                false,
            )?;
            b.bind("emission_color", emission, CHANNEL_RGB)?;
        }
        if let Some(normal) = mat.and_then(|m| m.normal_texture()) {
            let normal_tap = tap(
                normal.texture(),
                normal.tex_coord(),
                normal.extension_value("KHR_texture_transform"),
                false,
                false,
            )?;
            b.bind("geometry_normal", normal_tap, CHANNEL_RGB)?;
            b.set("geometry_normal", &[normal.scale()]);
        }
        if let Some(occlusion) = mat.and_then(|m| m.occlusion_texture()) {
            let occlusion_tap = tap(
                occlusion.texture(),
                occlusion.tex_coord(),
                occlusion.extension_value("KHR_texture_transform"),
                false,
                false,
            )?;
            b.bind_index(OCCLUSION_INPUT, occlusion_tap, 0)?;
            extra[2] = occlusion.strength();
        }
        // KHR_materials_transmission and _volume: transmission is tinted by the base colour,
        // or by the interior's attenuation colour over its distance when the volume has one.
        let transmission = mat.and_then(|m| m.transmission());
        let volume = mat.and_then(|m| m.volume());
        weight = transmission
            .as_ref()
            .map_or(0., |t| t.transmission_factor());
        b.set("transmission_weight", &[weight]);
        if let Some(info) = transmission.as_ref().and_then(|t| t.transmission_texture()) {
            let transmission_tap = tap(
                info.texture(),
                info.tex_coord(),
                info_transform(&info).as_ref(),
                false,
                false,
            )?;
            b.bind("transmission_weight", transmission_tap, 0)?;
        }
        let attenuation = volume
            .as_ref()
            .filter(|v| v.attenuation_distance().is_finite())
            .map(|v| (v.attenuation_color(), v.attenuation_distance()));
        if let Some((color, distance)) = attenuation {
            b.set("transmission_color", &color);
            b.set("transmission_depth", &[distance]);
        } else if weight > 0. {
            b.set("transmission_color", &base[..3]);
            if let Some(info) = pbr.as_ref().and_then(|p| p.base_color_texture()) {
                let base_tap = tap(
                    info.texture(),
                    info.tex_coord(),
                    info_transform(&info).as_ref(),
                    true,
                    alpha,
                )?;
                b.bind("transmission_color", base_tap, CHANNEL_RGB)?;
            }
        }
        // glTF surfaces are thin unless KHR_materials_volume gives them thickness.
        let thin = volume.as_ref().is_none_or(|v| v.thickness_factor() <= 0.);
        b.set("geometry_thin_walled", &[f32::from(u8::from(thin))]);
        // KHR clearcoat, sheen and specular, read from their JSON (typed features would hide them).
        let extension = |name: &str| mat.and_then(|m| m.extension_value(name));
        let number = |e: Option<&serde_json::Value>, key: &str| {
            e.and_then(|e| e.get(key)?.as_f64()).map(|v| v as f32)
        };
        let color = |e: Option<&serde_json::Value>, key: &str| {
            e.and_then(|e| serde_json::from_value::<[f32; 3]>(e.get(key)?.clone()).ok())
        };
        let mut json_tap =
            |e: Option<&serde_json::Value>, key: &str, srgb: bool| -> Result<Option<Tap>> {
                let Some(info) = e.and_then(|e| e.get(key)) else {
                    return Ok(None);
                };
                let index = info
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .context("glTF extension texture needs an index")? as usize;
                let texture = doc
                    .textures()
                    .nth(index)
                    .context("glTF extension texture index out of range")?;
                let tex_coord = info.get("texCoord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let transform = info.pointer("/extensions/KHR_texture_transform");
                Ok(Some(tap(texture, tex_coord, transform, srgb, true)?))
            };
        if let Some(coat) = extension("KHR_materials_clearcoat") {
            b.set(
                "coat_weight",
                &[number(Some(coat), "clearcoatFactor").unwrap_or(0.)],
            );
            b.set(
                "coat_roughness",
                &[number(Some(coat), "clearcoatRoughnessFactor").unwrap_or(0.)],
            );
            b.set("coat_ior", &[constants::GLTF_CLEARCOAT_IOR]);
            if let Some(coat_tap) = json_tap(Some(coat), "clearcoatTexture", false)? {
                b.bind("coat_weight", coat_tap, 0)?;
            }
            if let Some(roughness) = json_tap(Some(coat), "clearcoatRoughnessTexture", false)? {
                b.bind("coat_roughness", roughness, 1)?;
            }
        }
        if let Some(sheen) = extension("KHR_materials_sheen") {
            // glTF sheen has no weight: the colour carries the strength, so the weight is the
            // colour's maximum and the fuzz colour its normalised hue (same throughput as KHR).
            let sheen_color = color(Some(sheen), "sheenColorFactor").unwrap_or([0.; 3]);
            let fuzz = sheen_color.into_iter().fold(0., f32::max);
            b.set("fuzz_weight", &[fuzz]);
            if fuzz > 0. {
                b.set("fuzz_color", &sheen_color.map(|c| c / fuzz));
            }
            b.set(
                "fuzz_roughness",
                &[number(Some(sheen), "sheenRoughnessFactor").unwrap_or(0.)],
            );
            if let Some(color_tap) = json_tap(Some(sheen), "sheenColorTexture", true)? {
                b.bind("fuzz_color", color_tap, CHANNEL_RGB)?;
            }
            if let Some(roughness) = json_tap(Some(sheen), "sheenRoughnessTexture", false)? {
                b.bind("fuzz_roughness", roughness, 3)?;
            }
        }
        if let Some(specular) = extension("KHR_materials_specular") {
            b.set(
                "specular_weight",
                &[number(Some(specular), "specularFactor").unwrap_or(1.)],
            );
            b.set(
                "specular_color",
                &color(Some(specular), "specularColorFactor").unwrap_or([1.; 3]),
            );
            if let Some(weight) = json_tap(Some(specular), "specularTexture", false)? {
                b.bind("specular_weight", weight, 3)?;
            }
            if let Some(tint) = json_tap(Some(specular), "specularColorTexture", true)? {
                b.bind("specular_color", tint, CHANNEL_RGB)?;
            }
        }
    }
    // Unlit uses the existing emission primitive. The shared surface evaluator
    // transfers the sampled, vertex-coloured base to emission after decoding.
    if unlit {
        for name in ["base_weight", "specular_weight", "coat_weight", "fuzz_weight",
                     "transmission_weight", "subsurface_weight"] {
            b.set(name, &[0.]);
        }
        b.set("emission_color", &base[..3]);
        b.set("emission_luminance", &[1.]);
        extra[2] = 0.;
    }
    let cutoff = (alpha_mode == AlphaMode::Mask)
        .then(|| mat.and_then(|m| m.alpha_cutoff()).unwrap_or(0.5))
        .unwrap_or(-1.);
    let blend = alpha_mode == AlphaMode::Blend;
    let double_sided = mat.is_none_or(|m| m.double_sided());
    b.set("alpha_cutoff", &[cutoff]);
    b.record.extra = extra;
    b.record.tag[2] = u32::from(double_sided) * FLAG_DOUBLE_SIDED
        | u32::from(alpha) * FLAG_VERTEX_ALPHA
        | constants::FLAG_VERTEX_COLOR
        | if unlit { vork_slang::registry::FLAG_GLTF_UNLIT } else { 0 };
    if b.record.inputs[..OPENPBR.len()].iter().any(|v| v & constants::MATERIAL_TAP_INDEX_MASK != 0) {
        b.record.tag[2] |= vork_slang::registry::FLAG_SAMPLE_MATERIAL;
    }
    let mut pending = PendingMaterial {
        record: b.record,
        uv_sources: uv_sources.sources(),
        emission_animated: false,
        source_emission: false,
        taps: b.taps,
        alpha_mask: alpha_mode == AlphaMode::Mask,
        shading: 0,
        response_animated: 0,
        response_program: 0,
        unique: None,
        double_sided,
        transparent: !unlit && weight > 0.,
        opacity_animated: false,
        transmissive: !unlit && weight > 0.,
        lobes: false,
        lit: false,
        hair: false,
        physical_hair: false,
    };
    if blend || alpha_mode == AlphaMode::Mask {pending.integrate_opacity(None,pending.transmissive,false);}
    if alpha_mode == AlphaMode::Mask && !pending.transmissive {pending.record.extra[1]=3.;}
    Ok(pending)
}

pub(crate) fn material_sampler(
    d: &wgpu::Device,
    texture: Option<gltf::Texture<'_>>,
) -> wgpu::Sampler {
    use gltf::texture::{MagFilter, MinFilter, WrappingMode};
    let sampler = texture.map(|t| t.sampler());
    let wrap = |mode| match mode {
        WrappingMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        WrappingMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
        WrappingMode::Repeat => wgpu::AddressMode::Repeat,
    };
    let min = sampler
        .as_ref()
        .and_then(|s| s.min_filter())
        .unwrap_or(MinFilter::LinearMipmapLinear);
    let mag = sampler
        .as_ref()
        .and_then(|s| s.mag_filter())
        .unwrap_or(MagFilter::Linear);
    d.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("glTF sampler"),
        address_mode_u: wrap(
            sampler
                .as_ref()
                .map_or(WrappingMode::Repeat, |s| s.wrap_s()),
        ),
        address_mode_v: wrap(
            sampler
                .as_ref()
                .map_or(WrappingMode::Repeat, |s| s.wrap_t()),
        ),
        mag_filter: if mag == MagFilter::Nearest {
            wgpu::FilterMode::Nearest
        } else {
            wgpu::FilterMode::Linear
        },
        min_filter: if matches!(
            min,
            MinFilter::Nearest | MinFilter::NearestMipmapNearest | MinFilter::NearestMipmapLinear
        ) {
            wgpu::FilterMode::Nearest
        } else {
            wgpu::FilterMode::Linear
        },
        mipmap_filter: if matches!(
            min,
            MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear
        ) {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        lod_max_clamp: if matches!(min, MinFilter::Nearest | MinFilter::Linear) {
            0.
        } else {
            32.
        },
        ..Default::default()
    })
}

type Imported = (
    std::sync::Arc<gltf::Document>,
    std::sync::Arc<Vec<gltf::buffer::Data>>,
    Vec<Image>,
);

struct ImageSource {
    document: std::sync::Arc<gltf::Document>,
    buffers: std::sync::Arc<Vec<gltf::buffer::Data>>,
    base: PathBuf,
    /// The device's texture side (`texture_source::decode_format`).
    max_side: u32,
}
impl ImageSource {
    /// Decode image `index` from `encoded`, or from its source when not given.
    fn decode(&self, index: usize, encoded: Option<&[u8]>) -> Result<gltf::image::Data> {
        let image = self.document.images().nth(index).context("missing glTF image")?;
        let read;
        let encoded = match encoded {
            Some(encoded) => encoded,
            None => {
                read = encoded_image(&image, &self.buffers, Some(self.base.as_path()))?.0;
                &read
            }
        };
        decode_image_bytes(&image.source(), encoded, self.max_side)
    }
}

/// One install retains the existing decoder budget across imported assets.
/// Starting another source joins the previous group while GPU preparation continues.
#[derive(Default)]
pub(crate) struct ImageDecoders {
    handles: Vec<std::thread::JoinHandle<Result<()>>>,
}
impl ImageDecoders {
    pub(crate) fn finish(&mut self) -> Result<()> {
        let mut failure = None;
        for handle in std::mem::take(&mut self.handles) {
            let result = handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("image decoder worker panicked")));
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for ImageDecoders {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Metadata is enough for an existing GPU mip payload. Expand lossless pixels
/// only when an upload actually needs them, retaining the original decoder for
/// unsupported compressed formats, cold preparation and damaged cache entries.
pub(crate) struct Image {
    width: u32,
    height: u32,
    format: gltf::image::Format,
    data: std::cell::OnceCell<gltf::image::Data>,
    source: Option<(std::sync::Arc<ImageSource>, usize, PathBuf)>,
    pending: Option<std::sync::mpsc::Receiver<Result<gltf::image::Data>>>,
}
impl From<gltf::image::Data> for Image {
    fn from(data: gltf::image::Data) -> Self {
        Self {
            width: data.width,
            height: data.height,
            format: data.format,
            data: std::cell::OnceCell::from(data),
            source: None,
            pending: None,
        }
    }
}
impl Image {
    fn data(&self) -> Result<&gltf::image::Data> {
        if self.data.get().is_none() {
            let data = if let Some(pending) = &self.pending {
                pending.recv().context("image decoder worker panicked")??
            } else {
                let (source, index, path) = self.source.as_ref().context("missing image source")?;
                if let Some(data) = read_decoded(path) {
                    data
                } else {
                    let data = source.decode(*index, None)?;
                    let _ = write_decoded(path, &data);
                    data
                }
            };
            let _ = self.data.set(data);
        }
        Ok(self.data.get().unwrap())
    }
}

/// Read only the encoded source headers already in memory. Ready GPU mips do
/// not need the much larger lossless pixel cache to be read, hashed or expanded.
pub(crate) fn image_metadata(
    source: &gltf::image::Source<'_>,
    encoded: &[u8],
    max_side: u32,
) -> Result<(u32, u32, gltf::image::Format)> {
    let (width, height, color) = crate::texture_source::header(image_format(source)?, encoded, max_side)?;
    Ok((width, height, image_channel_format(color)?))
}

fn image_channel_format(color: image::ColorType) -> Result<gltf::image::Format> {
    Ok(match color {
        image::ColorType::L8 => gltf::image::Format::R8,
        image::ColorType::La8 => gltf::image::Format::R8G8,
        image::ColorType::Rgb8 => gltf::image::Format::R8G8B8,
        image::ColorType::Rgba8 => gltf::image::Format::R8G8B8A8,
        image::ColorType::L16 => gltf::image::Format::R16,
        image::ColorType::La16 => gltf::image::Format::R16G16,
        image::ColorType::Rgb16 => gltf::image::Format::R16G16B16,
        image::ColorType::Rgba16 => gltf::image::Format::R16G16B16A16,
        image::ColorType::Rgb32F => gltf::image::Format::R32G32B32FLOAT,
        image::ColorType::Rgba32F => gltf::image::Format::R32G32B32A32FLOAT,
        _ => bail!("unsupported image channel format"),
    })
}

/// The decoder for a glTF image: glTF owns URI reading; its MIME/data-URI/suffix
/// precedence is preserved here.
fn image_format(source: &gltf::image::Source<'_>) -> Result<image::ImageFormat> {
    let mime = match source {
        gltf::image::Source::View { mime_type, .. } => Some(*mime_type),
        gltf::image::Source::Uri { uri, mime_type } => uri
            .strip_prefix("data:")
            .and_then(|rest| rest.split_once(";base64,").map(|(mime, _)| mime))
            .or(*mime_type),
    };
    Ok(match (mime, source) {
        (Some("image/png"), _) => image::ImageFormat::Png,
        (Some("image/jpeg"), _) => image::ImageFormat::Jpeg,
        (None, gltf::image::Source::Uri { uri, .. }) => match uri.rsplit('.').next() {
            Some("png") => image::ImageFormat::Png,
            Some("jpg" | "jpeg") => image::ImageFormat::Jpeg,
            _ => return Err(gltf::Error::UnsupportedImageEncoding.into()),
        },
        _ => return Err(gltf::Error::UnsupportedImageEncoding.into()),
    })
}

/// Decode the same PNG/JPEG bytes used for content identity, without asking
/// glTF's source decoder to read or base64-decode those bytes again.
pub(crate) fn decode_image_bytes(
    source: &gltf::image::Source<'_>,
    encoded: &[u8],
    max_side: u32,
) -> Result<gltf::image::Data> {
    decoded_data(crate::texture_source::decode_format(image_format(source)?, encoded, max_side)?)
}

/// Decoded channels at their native type and precision.
pub(crate) fn decoded_data(decoded: image::DynamicImage) -> Result<gltf::image::Data> {
    Ok(gltf::image::Data {
        width: decoded.width(),
        height: decoded.height(),
        format: image_channel_format(decoded.color())?,
        pixels: decoded.into_bytes(),
    })
}

/// One diagnostic per unsupported extension per imported document, independent of
/// primitive reuse and of whether an authored material replaces an imported slot.
fn gltf_extension_warnings(document: &gltf::Document) -> Vec<String> {
    let mut counts = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for material in document.materials() {
        // Typed supported extensions are consumed by gltf and absent here.
        for extension in material.extensions().into_iter().flat_map(|e| e.keys()) {
            if !GLTF_OPENPBR_EXTENSIONS.contains(&extension.as_str()) {
                counts.entry(extension.clone()).or_default().0 += 1;
            }
        }
        if material.extension_value("KHR_materials_clearcoat")
            .is_some_and(|e| e.get("clearcoatNormalTexture").is_some())
        {
            counts.entry("KHR_materials_clearcoat".into()).or_default().0 += 1;
        }
    }
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if primitive.extension_value("KHR_materials_variants").is_some() {
                counts.entry("KHR_materials_variants".into()).or_default().1 += 1;
            }
        }
    }
    counts.into_iter().map(|(extension, (materials, primitives))| {
        let detail = match extension.as_str() {
            "KHR_materials_variants" => "not applied yet; using primitive materials",
            "KHR_materials_clearcoat" => "clearcoatNormalTexture is not applied yet (coat normals are not shaded)",
            _ => "not applied yet",
        };
        format!("{extension}: {detail} ({materials} materials, {primitives} primitives affected)")
    }).collect()
}

pub(crate) fn load_cached(
    path: &Path,
    cache: &disk_cache::Store,
    decoders: &mut ImageDecoders,
    max_side: u32,
) -> Result<(Imported, Vec<blake3::Hash>)> {
    decoders.finish()?;
    let gltf::Gltf { document, blob } = import::open(path)?;
    for warning in gltf_extension_warnings(&document) {
        crate::warnings::dropped("gltf_extension", serde_json::json!({"file":path,"extension":warning.split(':').next(),"materials":document.materials().filter_map(|m| m.extensions().map(|e|serde_json::json!({"index":m.index(),"name":m.name(),"extensions":e}))).collect::<Vec<_>>(),"primitive_variants":document.meshes().flat_map(|m|m.primitives()).filter_map(|p|p.extension_value("KHR_materials_variants").cloned()).collect::<Vec<_>>()}), warning);
    }
    let base = path.parent().unwrap_or(Path::new("./"));
    let buffers = std::sync::Arc::new(import::buffers(&document, Some(base), blob)?);
    let document = std::sync::Arc::new(document);
    let source = std::sync::Arc::new(ImageSource {
        document: document.clone(),
        buffers: buffers.clone(),
        base: base.to_owned(),
        max_side,
    });
    let count = document.images().len();
    // Retain the existing decoder budget. Publish source identities/headers before
    // pixel work so installation can feed the existing GPU preparation queue.
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8)
        .min(count);
    let (ready, batches) = std::sync::mpsc::channel();
    for worker in 0..workers {
        let source = source.clone();
        let cache = cache.clone();
        let ready = ready.clone();
        let path = path.to_owned();
        let handle = std::thread::spawn(move || -> Result<()> {
            let prepared = source
                .document
                .images()
                .skip(worker)
                .step_by(workers)
                .map(|image| {
                    let index = image.index();
                    prepare_cached(image, &source, &cache)
                        .map(|prepared| (index, prepared))
                        .with_context(|| format!("load {} image {index}", path.display()))
                })
                .collect::<Result<Vec<_>>>();
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = ready.send(Err(anyhow::anyhow!("{error:#}")));
                    return Err(error);
                }
            };
            let mut metadata = Vec::with_capacity(prepared.len());
            let mut tasks = Vec::new();
            for (index, (image, key, task)) in prepared {
                let pixels = task.map(|(pixels, task)| {
                    tasks.push(task);
                    pixels
                });
                metadata.push((index, image, key, pixels));
            }
            let _ = ready.send(Ok(metadata));
            drop(ready);
            let mut failure = None;
            for task in tasks {
                let result = source
                    .decode(task.index, Some(&task.encoded))
                    .with_context(|| format!("load {} image {}", path.display(), task.index));
                drop(task.encoded);
                match result {
                    Ok(data) => {
                        let _ = write_decoded(&task.path, &data);
                        let _ = task.pixels.send(Ok(data));
                    }
                    Err(error) => {
                        let _ = task.pixels.send(Err(anyhow::anyhow!("{error:#}")));
                        failure.get_or_insert(error);
                    }
                }
            }
            failure.map_or(Ok(()), Err)
        });
        decoders.handles.push(handle);
    }
    drop(ready);
    let mut images = Vec::with_capacity(count);
    for _ in 0..workers {
        let batch = match batches.recv() {
            Ok(batch) => batch?,
            Err(_) => {
                decoders.finish()?;
                bail!("image decoder returned no metadata");
            }
        };
        for (index, mut image, key, pixels) in batch {
            image.pending = pixels;
            images.push((index, (image, key)));
        }
    }
    images.sort_unstable_by_key(|(index, _)| *index);
    let images: Vec<_> = images.into_iter().map(|(_, image)| image).collect();
    let (images, keys): (Vec<_>, Vec<_>) = images.into_iter().unzip();
    Ok(((document, buffers, images), keys))
}

const DECODED_FORMATS: [gltf::image::Format; 10] = {
    use gltf::image::Format::*;
    [
        R8,
        R8G8,
        R8G8B8,
        R8G8B8A8,
        R16,
        R16G16,
        R16G16B16,
        R16G16B16A16,
        R32G32B32FLOAT,
        R32G32B32A32FLOAT,
    ]
};

struct DecodeTask<'a> {
    index: usize,
    encoded: std::borrow::Cow<'a, [u8]>,
    path: PathBuf,
    pixels: std::sync::mpsc::Sender<Result<gltf::image::Data>>,
}
type PreparedImage<'a> = (
    Image,
    blake3::Hash,
    Option<(
        std::sync::mpsc::Receiver<Result<gltf::image::Data>>,
        DecodeTask<'a>,
    )>,
);

/// An image's encoded bytes and the source settings that select its decoder.
/// glTF's URI reader gives the same percent-decoding, file and data URI
/// semantics; without a base, external URIs are refused.
pub(crate) fn encoded_image<'a>(
    image: &gltf::Image<'_>,
    buffers: &'a [gltf::buffer::Data],
    base: Option<&Path>,
) -> Result<(std::borrow::Cow<'a, [u8]>, String)> {
    use std::borrow::Cow;
    Ok(match &image.source() {
        gltf::image::Source::View { view, mime_type } => {
            (Cow::Borrowed(import::view_bytes(buffers, view)?), mime_type.to_string())
        }
        gltf::image::Source::Uri { uri, mime_type } => {
            let data = crate::failure::contain("the glTF image reader", || {
                Ok(gltf::buffer::Data::from_source(gltf::buffer::Source::Uri(uri), base)?)
            })?;
            // The image decoder's format selection uses MIME, data URI MIME or file suffix.
            let setting = if uri.starts_with("data:") {
                uri.split(";base64,").next().unwrap_or(uri).to_string()
            } else {
                format!("{mime_type:?}:{}", uri.rsplit('.').next().unwrap_or(""))
            };
            (Cow::Owned(data.0), setting)
        }
    })
}

fn prepare_cached<'a>(
    image: gltf::Image<'_>,
    context: &'a std::sync::Arc<ImageSource>,
    cache: &disk_cache::Store,
) -> Result<PreparedImage<'a>> {
    let (encoded, settings) = encoded_image(&image, &context.buffers, Some(context.base.as_path()))?;
    let mut key = blake3::Hasher::new();
    // The store supplies format identity. Keep source settings and encoded
    // content at the existing granularity; never rehash decoded pixels.

    key.update(settings.as_bytes());
    key.update(&[0]);
    key.update(&encoded);
    let key = key.finalize();
    let path = cache.path("decoded-texture", 3, key);
    let (width, height, format) = image_metadata(&image.source(), &encoded, context.max_side)?;
    let task = if path.is_file() {
        None
    } else {
        let (pixels, receiver) = std::sync::mpsc::channel();
        Some((
            receiver,
            DecodeTask {
                index: image.index(),
                encoded,
                path: path.clone(),
                pixels,
            },
        ))
    };
    Ok((
        Image {
            width,
            height,
            format,
            data: std::cell::OnceCell::new(),
            source: Some((context.clone(), image.index(), path)),
            pending: None,
        },
        key,
        task,
    ))
}

fn read_decoded(path: &Path) -> Option<gltf::image::Data> {
    let encoded = disk_cache::read_mapped(path)?;
    let header = encoded.get(..16)?;
    let width = u32::from_le_bytes(header[..4].try_into().ok()?);
    let height = u32::from_le_bytes(header[4..8].try_into().ok()?);
    let index = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
    let format = *DECODED_FORMATS.get(index)?;
    let bytes_per_pixel = [1u64, 2, 3, 4, 2, 4, 6, 8, 12, 16][index];
    let expected = u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(bytes_per_pixel)?;
    let length = u32::from_le_bytes(header[12..16].try_into().ok()?);
    if width == 0 || height == 0 || expected != u64::from(length) {
        return None;
    }
    let pixels = lz4_flex::decompress_size_prepended(&encoded[12..]).ok()?;
    if pixels.len() as u64 != expected {
        return None;
    }
    Some(gltf::image::Data {
        pixels,
        format,
        width,
        height,
    })
}
#[cfg(test)]
#[test]
fn compact_decoded_roundtrip() {
    for (index, format) in DECODED_FORMATS.iter().enumerate() {
        let bytes_per_pixel = [1, 2, 3, 4, 2, 4, 6, 8, 12, 16][index];
        let data = gltf::image::Data {
            width: 16,
            height: 8,
            format: *format,
            pixels: (0..16 * 8 * bytes_per_pixel)
                .map(|i| (i % 251) as u8)
                .collect(),
        };
        let path =
            std::env::temp_dir().join(format!("vork-decoded-{}-{index}.entry", std::process::id()));
        write_decoded(&path, &data).unwrap();
        let decoded = read_decoded(&path).unwrap();
        assert_eq!(
            (decoded.width, decoded.height, decoded.format),
            (data.width, data.height, data.format)
        );
        assert_eq!(decoded.pixels, data.pixels);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
#[test]
fn encoded_decode_matches_gltf_source_channels_and_mime() -> Result<()> {
    let root = std::env::temp_dir().join(format!("vork-encoded-source-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    for (name, image, encoding) in [
        (
            "sixteen bit.png",
            image::DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                2,
                1,
                image::Rgba([17, 256, 65000, 32768]),
            )),
            image::ImageFormat::Png,
        ),
        (
            "rgb.jpg",
            image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(
                2,
                1,
                image::Rgb([17, 42, 93]),
            )),
            image::ImageFormat::Jpeg,
        ),
    ] {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image.write_to(&mut encoded, encoding)?;
        let bytes = encoded.into_inner();
        std::fs::write(root.join(name), &bytes)?;
        let uri = name.replace(' ', "%20");
        let source = gltf::image::Source::Uri {
            uri: &uri,
            mime_type: None,
        };
        let expected = gltf::image::Data::from_source(source.clone(), Some(&root), &[])?;
        // The URI reader used for identity pads buffers to four-byte alignment.
        let padded = gltf::buffer::Data::from_source(gltf::buffer::Source::Uri(&uri), Some(&root))?;
        let decoded = decode_image_bytes(&source, &padded.0, crate::fuzz::MAX_SIDE)?;
        assert_eq!(
            (decoded.width, decoded.height, decoded.format),
            (expected.width, expected.height, expected.format)
        );
        assert_eq!(decoded.pixels, expected.pixels);
        let mime = if encoding == image::ImageFormat::Png {
            "image/png"
        } else {
            "image/jpeg"
        };
        let document = gltf::Gltf::from_slice(&serde_json::to_vec(&serde_json::json!({
            "asset":{"version":"2.0"}, "buffers":[{"byteLength":bytes.len()}],
            "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],
            "images":[{"bufferView":0,"mimeType":mime}]
        }))?)?
        .document;
        let source = document.images().next().unwrap().source();
        let expected = gltf::image::Data::from_source(
            source.clone(),
            Some(&root),
            &[gltf::buffer::Data(bytes.clone())],
        )?;
        assert_eq!(decode_image_bytes(&source, &bytes, crate::fuzz::MAX_SIDE)?.pixels, expected.pixels);
        let source = gltf::image::Source::Uri {
            uri: "data:image/png;base64,ignored",
            mime_type: Some("image/jpeg"),
        };
        if encoding == image::ImageFormat::Png {
            assert_eq!(decode_image_bytes(&source, &bytes, crate::fuzz::MAX_SIDE)?.pixels, expected.pixels);
        }
        let invalid = gltf::image::Source::Uri {
            uri: &uri,
            mime_type: Some("png"),
        };
        assert!(decode_image_bytes(&invalid, &bytes, crate::fuzz::MAX_SIDE).is_err());
    }
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[cfg(test)]
#[test]
fn decoded_header_rejects_short_pixel_block() -> Result<()> {
    let path = std::env::temp_dir().join(format!("vork-decoded-short-{}.entry", std::process::id()));
    let mut header = [0u8; 12];
    header[..4].copy_from_slice(&2u32.to_le_bytes());
    header[4..8].copy_from_slice(&1u32.to_le_bytes());
    header[8..].copy_from_slice(&3u32.to_le_bytes());
    let mut pixels = lz4_flex::compress_prepend_size(&[0, 1, 2, 3]);
    pixels[..4].copy_from_slice(&8u32.to_le_bytes());
    disk_cache::write_parts(&path, &[&header, &pixels])?;
    assert!(read_decoded(&path).is_none());
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
#[test]
fn cached_image_defers_pixels_and_repairs_corrupt_entries() -> Result<()> {
    let root = std::env::temp_dir().join(format!("vork-lazy-image-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let path = root.join("pixels.entry");
    let pixels = vec![17, 42, 93, 255, 255, 0, 81, 128];
    image::save_buffer(
        root.join("source.png"),
        &pixels,
        2,
        1,
        image::ColorType::Rgba8,
    )?;
    let document =
        gltf::Gltf::from_slice(br#"{"asset":{"version":"2.0"},"images":[{"uri":"source.png"}]}"#)?
            .document;
    let source = std::sync::Arc::new(ImageSource {
        document: std::sync::Arc::new(document),
        buffers: std::sync::Arc::new(vec![]),
        base: root.clone(),
        max_side: crate::fuzz::MAX_SIDE,
    });
    let data = gltf::image::Data {
        pixels: pixels.clone(),
        width: 2,
        height: 1,
        format: gltf::image::Format::R8G8B8A8,
    };
    write_decoded(&path, &data)?;
    let make_image = || -> Result<Image> {
        let png = gltf::image::Source::Uri { uri: "source.png", mime_type: None };
        let (width, height, format) =
            image_metadata(&png, &std::fs::read(root.join("source.png"))?, crate::fuzz::MAX_SIDE)?;
        Ok(Image {
            width,
            height,
            format,
            data: std::cell::OnceCell::new(),
            source: Some((source.clone(), 0, path.clone())),
            pending: None,
        })
    };
    let image = make_image()?;
    assert_eq!(
        (image.width, image.height, image.format),
        (2, 1, data.format)
    );
    assert!(image.data.get().is_none());
    assert_eq!(image.data()?.pixels, pixels);
    assert!(std::ptr::eq(image.data()?, image.data()?));
    drop(image);
    // A damaged unused decoded entry does not prevent a ready GPU mip hit.
    // When pixels are requested, the ordinary integrity check must miss and
    // the original glTF decoder must repair the entry.
    std::fs::write(&path, b"damaged decoded cache")?;
    let repaired = make_image()?;
    assert!(repaired.data.get().is_none());
    assert_eq!(repaired.data()?.pixels, pixels);
    assert_eq!(
        read_decoded(&path).context("repaired cache")?.pixels,
        pixels
    );
    drop(repaired);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn write_decoded(path: &Path, data: &gltf::image::Data) -> std::io::Result<()> {
    let format = DECODED_FORMATS
        .iter()
        .position(|f| *f == data.format)
        .unwrap() as u32;
    // Keep the small metadata separate: appending it to a cloned pixel Vec
    // would allocate/copy the entire image and can grow that allocation again.
    let mut header = [0u8; 12];
    header[..4].copy_from_slice(&data.width.to_le_bytes());
    header[4..8].copy_from_slice(&data.height.to_le_bytes());
    header[8..].copy_from_slice(&format.to_le_bytes());
    let pixels = lz4_flex::compress_prepend_size(&data.pixels);
    disk_cache::write_parts(path, &[&header, &pixels])
}

/// Byte offset of OpenPBR parameter `parameter` (index into `v2::OPENPBR`) of material
/// record `slot` in the record storage, and its width in f32 words.
pub(crate) fn value_location(slot: usize, parameter: usize) -> Option<(u64, usize)> {
    let p = OPENPBR.get(parameter)?;
    Some({
        (
            (slot * std::mem::size_of::<MaterialRecord>()
                + std::mem::offset_of!(MaterialRecord, values)
                + value_word(parameter) * 4) as u64,
            value_width(p.kind),
        )
    })
}

/// Targets of one material-input track, read from the install-time record storage.
pub(crate) fn track_targets(
    (name, input): (&str, &str),
    slots: &[usize],
    records: &[u8],
) -> Result<Vec<(usize, usize, Vec<f32>)>> {
    let path = format!("[\"materials\",{name:?},\"inputs\",{input:?}]");
    let parameter = ::vork_scene::v2::validate_openpbr_input(input)
        .map(|p| OPENPBR.iter().position(|q| q.name == p.name).unwrap())
        .with_context(|| format!("track {path}"))?;
    ensure!(
        !matches!(
            OPENPBR[parameter].kind,
            OpenPbrKind::Bool
        ),
        "track {path}: {input} is structural (thin versus solid transport)"
    );
    slots
        .iter()
        .map(|&slot| {
            let (offset, width) = value_location(slot, parameter).unwrap();
            let bytes = records
                .get(offset as usize..offset as usize + width * 4)
                .with_context(|| format!("track {path}: record {slot} is not installed"))?;
            Ok((
                slot,
                parameter,
                bytemuck::pod_collect_to_vec::<u8, f32>(bytes),
            ))
        })
        .collect()
}

/// Materials whose inputs a track animates, with those inputs. Their records must stay
/// distinct (never deduplicated) so per-frame writes reach only them.
pub(crate) fn animated_inputs(
    scene: &::vork_scene::v2::Scene,
) -> Result<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> {
    let mut out = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for target in scene.track_targets()?.into_iter().flatten() {
        if let ::vork_scene::TrackTarget::Material { material, input } = target {
            out.entry(material).or_default().insert(input);
        }
    }
    Ok(out)
}

/// A std/openpbr material as a record, checked against what this renderer applies, with
/// the image each texture tap reads. Pure: images resolve to table words in
/// [`install_openpbr`] through `word`.
pub(crate) struct OpenPbrRecord {
    pub(crate) capabilities: vork_slang::ProgramCapabilities,
    pub(crate) builder: RecordBuilder,
    pub(crate) has_opacity: bool,
    pub(crate) transmissive: bool,
    pub(crate) shading: u8,
}
impl OpenPbrRecord {
    pub(crate) fn spatial_subsurface(&self) -> bool {
        self.capabilities.spatial_subsurface
    }
}
pub(crate) fn openpbr_values(
    scene: &::vork_scene::v2::Scene,
    name: Option<&str>,
    animated: &std::collections::BTreeSet<String>,
    mut word: impl FnMut(&str) -> Result<u64>,
) -> Result<OpenPbrRecord> {
    use ::vork_scene::v2::{Channel, Factor, Input};
    let mut b = RecordBuilder::openpbr();
    let default_material = ::vork_scene::v2::Material {
        program: "std/openpbr".into(),
        inputs: Default::default(),
        double_sided: true,
        opacity: None,
    };
    let material = match name {
        None => &default_material,
        Some(name) => scene.materials.get(name)
            .with_context(|| format!("unknown material {name}"))?,
    };
    let name = name.unwrap_or("default OpenPBR");
    ensure!(
        material.program == "std/openpbr",
        "material {name}: install program materials through programs.rs"
    );
    for input in animated {
        let (index, p) = param(input);
        if p.kind == OpenPbrKind::Vector3 {
            b.record.inputs[index] |= 1u32 << 31;
            if !material.inputs.contains_key(input) {
                b.record.values[value_word(index)..value_word(index) + 3].fill(0.);
            }
        }
    }
    for (input, value) in &material.inputs {
        let p = ::vork_scene::v2::validate_openpbr_input(input).with_context(|| format!("material {name}"))?;
        let image = match value {
            Input::Number(x) => {
                b.set(input, &[*x]);
                continue;
            }
            Input::Vector(v) => {
                b.set(input, v);
                continue;
            }
            Input::Bool(on) => {
                b.set(input, &[f32::from(u8::from(*on))]);
                continue;
            }
            Input::Attribute(binding) => {
                ensure!(
                    matches!(p.kind, OpenPbrKind::Float | OpenPbrKind::Color3),
                    "material {name}: {input} attribute sources are not implemented for this input"
                );
                let names = crate::attributes::names(scene)?;
                let slot = names
                    .iter()
                    .position(|n| n == &binding.attribute)
                    .expect("bound attribute installed");
                let channel = match (&binding.channel, p.kind) {
                    (Some(Channel::R), _) | (None, OpenPbrKind::Float) => 0,
                    (Some(Channel::G), _) => 1,
                    (Some(Channel::B), _) => 2,
                    (Some(Channel::A), _) => 3,
                    (Some(Channel::Rgb) | None, OpenPbrKind::Color3) => CHANNEL_RGB,
                    _ => anyhow::bail!(
                        "material {name}: {input}: scalar attributes require r, g, b or a"
                    ),
                };
                let mut tap = Tap::plain(0);
                tap.uv = constants::MATERIAL_ATTRIBUTE_FLAG | (slot as u32) << 8;
                b.bind(input, tap, channel)?;
                match &binding.factor {
                    Some(Factor::Number(x)) => b.set(input, &[*x]),
                    Some(Factor::Vector(v)) => b.set(input, v),
                    None => b.set(input, &vec![1.; value_width(p.kind)]),
                }
                continue;
            }
            Input::Texture(binding) => binding.image.as_str(),
            other => anyhow::bail!("material {name}: {input}: std/openpbr does not take {}", serde_json::to_string(other)?),
        };
        ensure!(
            p.kind != OpenPbrKind::Bool,
            "material {name}: {input} textures are not implemented yet"
        );
        let image_def = scene
            .images
            .get(image)
            .with_context(|| format!("material {name}: unknown image {image}"))?;
        // The file default decodes PNG/JPEG as sRGB; data inputs need linear values.
        ensure!(
            p.kind == OpenPbrKind::Color3 || !image_def.is_srgb(),
            "material {name}: {input} reads data, so image {image} needs color_space \"Non-Color\""
        );
        let channel = if p.kind == OpenPbrKind::Float { 0 } else { CHANNEL_RGB };
        b.bind(input, Tap::plain(word(image)?), channel)
            .with_context(|| format!("material {name}"))?;
        if p.kind == OpenPbrKind::Vector3 { b.set(input, &[1.]); }
        else { b.set(input, &vec![1.; value_width(p.kind)]); }

    }
    // Normal/tangent tracks store ordinary tangent-space XYZ values.
    for input in animated {
        let (index, p) = param(input);
        if p.kind == OpenPbrKind::Vector3 {
            b.record.inputs[index] |= 1u32 << 31;
        }
    }
    let tracked = |input: &str| animated.contains(input);
    let opacity = b.get("geometry_opacity")[0];
    let has_opacity = opacity < 1.
        || b.bound("geometry_opacity")
        || b.bound("alpha_cutoff") || tracked("alpha_cutoff") || b.get("alpha_cutoff")[0] >= 0.
        || tracked("geometry_opacity");
    let value = |name| b.get(name)[0];
    let transmission = value("transmission_weight") > 0. || tracked("transmission_weight");
    let active_subsurface = value("subsurface_weight") > 0. || b.bound("subsurface_weight") || tracked("subsurface_weight");
    let capabilities = vork_slang::ProgramCapabilities { subsurface: active_subsurface, spatial_subsurface: active_subsurface && (value("geometry_thin_walled")<0.5 || b.bound("geometry_thin_walled") || tracked("geometry_thin_walled")), ..Default::default() };
    Ok(OpenPbrRecord {
        capabilities,
        has_opacity,
        transmissive: transmission,
        shading: (if tracked("coat_weight") || tracked("fuzz_weight") {
            SHADING_LAYERS
        } else {
            0
        }) | (if tracked("base_metalness") {
            SHADING_METAL
        } else {
            0
        }),
        builder: b,
    })
}

/// An authored or default std/openpbr material as a record with its textures in the scene
/// table. No per-object override carries any of it. `animated` names the inputs tracks
/// write: they reserve blending, transmission or shader features, and keep the record distinct.
pub(crate) fn install_openpbr(
    t: &mut TextureInstaller,
    scene: &::vork_scene::v2::Scene,
    name: Option<&str>,
    base: &Path,
    animated: &std::collections::BTreeSet<String>,
) -> Result<PendingMaterial> {
    let values = openpbr_values(scene, name, animated, |image| {
        let image_def = &scene.images[image];
        let (view, sampler) = t.scene_image(image, image_def, base)?;
        Ok(t.slot(view, sampler))
    })?;
    let record = values.builder.record;
    let material = name.and_then(|name| scene.materials.get(name));
    let mut pending = PendingMaterial {
        uv_sources: [0, 1],
        emission_animated: animated.contains("emission_luminance") || animated.contains("emission_color"),
        source_emission: false,
        record,
        taps: values.builder.taps,
        alpha_mask: false,
        double_sided: false,
        transparent: values.transmissive,
        opacity_animated: animated.contains("geometry_opacity") || animated.contains("alpha_cutoff"),
        transmissive: values.transmissive,
        lobes: false,
        lit: false,
        hair: false,
        physical_hair: false,
        shading: values.shading,
        response_program: 0,
        response_animated: if animated.contains("specular_roughness_anisotropy") {FEATURE_ANISOTROPY} else {0}
            | if animated.contains("thin_film_weight") {FEATURE_FILM} else {0},
        unique: name.filter(|_| !animated.is_empty()).map(str::to_owned),
    };
    pending.coverage(
        material.is_none_or(|m| m.double_sided),
    );
    if values.has_opacity {
        pending.integrate_opacity(material.and_then(|m|m.opacity), values.transmissive, false);
    }
    if pending.record.tag[0]==0 && pending.record.get("alpha_cutoff")[0]>=0. && !pending.record.bound("alpha_cutoff") && !animated.contains("alpha_cutoff") && !pending.transmissive {pending.record.extra[1]=3.;}
    Ok(pending)
}

/// The same record layout and closure evaluation as Slang modules, checked in as
/// shaders/generated/{material_layout,openpbr_inputs}.slang (the build compiles
/// them; `generated_slang_modules_are_current` keeps them equal to this).
#[cfg(test)]
pub(crate) fn input_slang() -> [(&'static str, String); 2] {
    use std::fmt::Write;
    let index = |name| param(name).0;
    let word = |name| value_word(index(name));
    let layout = format!(
        "// Generated by materials.rs input_slang(): the material record and the inputs shading reads directly.\n\
         import vork_constants;\n\
         struct MaterialRecord {{ uint4 tag; float4 extra; uint4 inputs[{}]; float4 values[{}]; }};\n\
         static const uint MATERIAL_OPACITY={}u;static const uint MATERIAL_OPACITY_WORD={}u;\n\
         static const uint MATERIAL_CUTOFF={}u;static const uint MATERIAL_CUTOFF_WORD={}u;\n\
         static const uint MATERIAL_THIN={}u;static const uint MATERIAL_THIN_WORD={}u;static const uint MATERIAL_TRANSMISSION={}u;static const uint MATERIAL_TRANSMISSION_WORD={}u;\n\
         static const uint MATERIAL_COAT_NORMAL={}u;static const uint MATERIAL_COAT_NORMAL_WORD={}u;static const uint MATERIAL_NORMAL={}u;static const uint MATERIAL_NORMAL_WORD={}u;static const uint MATERIAL_TANGENT={}u;static const uint MATERIAL_TANGENT_WORD={}u;static const uint MATERIAL_OCCLUSION={}u;\n",
        constants::MATERIAL_INPUT_WORDS / 4,
        VALUE_WORDS / 4,
        index("geometry_opacity"),
        word("geometry_opacity"),
        index("alpha_cutoff"), word("alpha_cutoff"),
        index("geometry_thin_walled"),
        word("geometry_thin_walled"),
        index("transmission_weight"),
        word("transmission_weight"),
        index("geometry_coat_normal"),
        word("geometry_coat_normal"),
        index("geometry_normal"),
        word("geometry_normal"),
        index("geometry_tangent"),
        word("geometry_tangent"),
        OCCLUSION_INPUT
    );
    let mut s = String::from(
        "// Generated by materials.rs input_slang(): every OpenPBR input from its record word or texture tap.\n\
         import vork_constants;\nimport closure;\nimport vork_material_table;\nimport material_layout;\nimport variant;\n\
         Closure material_closure<V:IVariant>(uint r, MaterialUv m, float3 normal, float3 tangent) {\n    var c=closure_defaults(normal,tangent);\n",
    );
    for (i, p) in OPENPBR.iter().enumerate().filter(|(_,p)| p.closure) {
        let w = value_word(i);
        let line = match p.kind {
            OpenPbrKind::Vector3 => continue,
            OpenPbrKind::Bool => format!("material_value(r,{w}u)>.5"),
            OpenPbrKind::Float => format!("material_float<V>(r,{i}u,{w}u,m)"),
            OpenPbrKind::Color3 => format!("material_color<V>(r,{i}u,{w}u,m)"),
        };
        let line = match p.name {
            "specular_roughness_anisotropy" => format!("V.has_anisotropy ? {line} : 0."),
            "thin_film_weight" => format!("V.has_film ? {line} : 0."),
            _ => line,
        };
        writeln!(s, "    c.{}={line};", p.name).unwrap();
    }
    s.push_str("    return c;\n}\n");
    let split=s.find("Closure material_closure").unwrap();
    let (imports,body)=s.split_at(split);
    let mut camera=String::from("Closure material_closure<V:IVariant>(uint r, MaterialUv m, float3 normal, float3 tangent) {\n    var c=closure_defaults(normal,tangent);\n");
    for p in OPENPBR.iter().filter(|p| p.closure) {
        let w=word(p.name);
        let value=match p.kind {
            OpenPbrKind::Vector3=>continue,
            OpenPbrKind::Bool=>format!("material_value(r,{w}u)>.5"),
            OpenPbrKind::Float=>format!("material_value(r,{w}u)"),
            OpenPbrKind::Color3=>format!("material_constant_color(r,{w}u)"),
        };
        let value=match p.name {
            "specular_roughness_anisotropy"=>format!("V.has_anisotropy ? {value} : 0."),
            "thin_film_weight"=>format!("V.has_film ? {value} : 0."),_=>value,
        };
        writeln!(camera,"    c.{}={value};",p.name).unwrap();
    }
    writeln!(camera,"    // Runtime sockets share one texture evaluator; no per-input lighting or shader variants.\n    [loop] for(uint input=0u;input<=MATERIAL_OPACITY;input++) {{\n        let b=material_byte<V>(r,input);if((b&REG_MATERIAL_TAP_INDEX_MASK)==0u) {{continue;}}\n        if((input=={}u && !V.has_anisotropy) || (input=={}u && !V.has_film)) {{continue;}}\n        let texel=material_sample(material_tap(r,b&REG_MATERIAL_TAP_INDEX_MASK,m));\n        let scalar=texel[min(b>>28u,3u)];let color=select((b>>28u)==4u,texel.rgb,float3(scalar));\n        switch(input) {{",index("specular_roughness_anisotropy"),index("thin_film_weight")).unwrap();
    for (i,p) in OPENPBR.iter().enumerate().filter(|(_,p)| p.closure) {
        let multiplier=match p.kind {OpenPbrKind::Float=>"scalar",OpenPbrKind::Color3=>"color",_=>continue};
        writeln!(camera,"        case {i}u:c.{}*={multiplier};break;",p.name).unwrap();
    }
    camera.push_str("        }\n    }\n    return c;\n}\n");
    let s=format!("{imports}#ifdef VORK_CAMERA_SPLIT\n{camera}#else\n{body}#endif\n");
    [("material_layout", layout), ("openpbr_inputs", s)]
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_opacity_requires_static_table_proof() -> Result<()> {
        let mut material = PendingMaterial::program(0, false);
        let mut builder = RecordBuilder::openpbr();
        builder.set("geometry_opacity", &[0.]);
        material.record = builder.record;
        assert!(material.clone().finish().zero_opacity);
        material.opacity_animated = true;
        assert!(!material.clone().finish().zero_opacity);
        material.opacity_animated = false;
        material.record.tag[2] |= constants::FLAG_VERTEX_ALPHA;
        assert!(!material.clone().finish().zero_opacity);
        material.record.tag[2] &= !constants::FLAG_VERTEX_ALPHA;
        material.record.tag[0] = 1;
        assert!(!material.clone().finish().zero_opacity);
        material.record.tag[0] = 0;
        builder.bind("geometry_opacity", Tap::plain(0), 3)?;
        material.record = builder.record;
        assert!(!material.clone().finish().zero_opacity);
        builder = RecordBuilder::openpbr();
        builder.set("geometry_opacity", &[f32::MIN_POSITIVE]);
        material.record = builder.record;
        assert!(!material.finish().zero_opacity);
        Ok(())
    }
    #[test]
    fn mip_chain_preserves_odd_extent_premultiplied_coverage() -> Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        let (pipelines,sampler)=mip_resources(&r);
        let pipeline=&pipelines[2];
        let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {
            label:Some("mip area readback"),size:256,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false,
        });
        for [width,height] in [[3u32,1u32],[3,3],[480,270]] {
            let levels=32-width.max(height).leading_zeros();
            let texture=crate::gpu_memory::texture(&r.device,&wgpu::TextureDescriptor {
                label:Some("mip area test"),size:extent(width,height),mip_level_count:levels,sample_count:1,
                dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba16Float,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::COPY_SRC,view_formats:&[],
            });
            let pixels:Vec<u16>=(0..width*height).flat_map(|p| {
                let value=half::f16::from_f32(if p%width%3==1 {0.} else {1.}).to_bits();
                [value;4]
            }).collect();
            r.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
                bytemuck::cast_slice(&pixels),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(width*8),rows_per_image:Some(height)},extent(width,height));
            let mut encoder=r.device.create_command_encoder(&Default::default());
            MipChain::new(&r.device,&texture,None,pipeline,sampler).encode(&mut encoder,None);
            encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:levels-1,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
                wgpu::TexelCopyBufferInfo {buffer:&readback,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(256),rows_per_image:Some(1)}},extent(1,1));
            let submission=crate::command_finish::Segments::submit(&r.queue, encoder);
            let (tx,rx)=std::sync::mpsc::channel();
            readback.map_async(wgpu::MapMode::Read,..,move |value| {let _=tx.send(value);});
            r.wait(Some(submission))?;rx.recv()??;
            let bytes=readback.get_mapped_range(..);
            for (c,&bits) in bytemuck::cast_slice::<u8,u16>(&bytes[..8]).iter().enumerate() {
                let value=half::f16::from_bits(bits).to_f32();
                assert!((value-2./3.).abs()<0.003,"{width}x{height} channel {c}: {value}");
            }
            drop(bytes);readback.unmap();
        }
        Ok(())
    }

    #[test]
    fn gltf_alpha_modes_keep_coverage_and_distinct_residency() -> Result<()> {
        let Some(r) = crate::test_renderer()? else {
            return Ok(());
        };
        let render_images = Default::default();
        let mut textures = TextureInstaller::new(&r, &render_images);
        let doc = gltf::Gltf::from_slice(
            br#"{
            "asset":{"version":"2.0"},
            "images":[{"uri":"alpha.png"}],"textures":[{"source":0}],
            "materials":[
                {"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},
                {"alphaMode":"MASK","pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},
                {"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}
            ]
        }"#,
        )?
        .document;
        let pixels: Vec<_> = (0..16).flat_map(|i| [17, 42, 93, (i * 17) as u8]).collect();
        let key = blake3::hash(&pixels);
        let images = [gltf::image::Data {
            pixels,
            format: gltf::image::Format::R8G8B8A8,
            width: 4,
            height: 4,
        }.into()];
        for material in doc.materials() {
            let installed=install_gltf(&mut textures, &doc, Some(&material), &images, &[key])?;
            assert_eq!(installed.record.tag[0],0,"ordinary glTF stays in the shared material table");
            assert_ne!(installed.record.tag[2]&vork_slang::registry::FLAG_SAMPLE_MATERIAL,0,"textured imports retain covered-sample evaluation");
            assert!(installed.record.bound("base_color"),"imported texture taps stay in the table");
            assert!(!installed.transparent,"non-transmitting imports use bounded camera visibility");
            assert_eq!(installed.alpha_mask,material.alpha_mode()!=gltf::material::AlphaMode::Opaque);
            assert_eq!(installed.record.extra[1],match material.alpha_mode() {gltf::material::AlphaMode::Blend=>2.,gltf::material::AlphaMode::Mask=>3.,gltf::material::AlphaMode::Opaque=>0.});
        }
        let resident = textures.resident.borrow();
        assert!(resident.contains_key(&(key, true, false)));
        assert!(resident.contains_key(&(key, true, true)));
        assert_eq!(resident.len(), 2);
        r.errors.check()?;
        Ok(())
    }

    /// Regenerate with VORK_BLESS=1 cargo test -p vork-render generated_slang.
    #[test]
    fn generated_slang_modules_are_current() {
        for (name, text) in input_slang() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("shaders/generated/{name}.slang"));
            if std::env::var_os("VORK_BLESS").is_some() {
                std::fs::write(&path, &text).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(&path).unwrap_or_default(),
                text,
                "{} is stale; regenerate with VORK_BLESS=1 cargo test -p vork-render generated_slang",
                path.display()
            );
        }
    }
    use ::vork_scene::v2::Scene;
    #[test]
    fn every_input_has_a_constant_and_a_tap_byte() {
        let words = std::mem::size_of_val(&MaterialRecord::zeroed().values) / 4;
        for (index, p) in OPENPBR.iter().enumerate() {
            assert!(
                value_word(index) + value_width(p.kind) <= words,
                "{}",
                p.name
            );
        }
        assert!(OCCLUSION_INPUT < constants::MATERIAL_INPUT_WORDS * 4);
        let base = OPENPBR.iter().position(|p| p.name == "base_color").unwrap();
        assert_eq!(
            value_location(2, base),
            Some((
                (2 * std::mem::size_of::<MaterialRecord>()
                    + std::mem::offset_of!(MaterialRecord, values)
                    + value_word(base) * 4) as u64,
                3
            ))
        );
    }
    #[test]
    fn gltf_source_uv_indices_resolve_independently_of_packed_channels() -> Result<()> {
        let mut sources = GltfUvSources::default();
        let third = sources.bind(2)?;
        assert_eq!(third & constants::FLAG_UV_SECOND, 0);
        assert_ne!(third & constants::FLAG_UV_BASIS, 0);
        assert_eq!(sources.bind(2)?, third);
        assert_eq!(sources.bind(7)? & constants::FLAG_UV_SECOND, 1);
        assert_eq!(sources.sources(), [2, 7]);
        assert!(sources.bind(0).unwrap_err().to_string().contains("two distinct UV sets"));
        let mut sources = GltfUvSources::default();
        assert_eq!(sources.bind(1)? & constants::FLAG_UV_SECOND, 1);
        assert_eq!(sources.bind(0)?, 0);
        assert_eq!(sources.sources(), [0, 1]);
        let transform = serde_json::json!({"texCoord":7,"scale":[2.,3.],"offset":[0.2,0.4],"rotation":0.5});
        let (set, scale, rotation, offset) = gltf_uv(0, Some(&transform))?;
        assert_eq!((set, scale, rotation, offset), (7, [2.,3.], 0.5, [0.2,0.4]));
        Ok(())
    }

    fn scene(materials: serde_json::Value) -> Scene {
        vork_scene::v2::from_test_value(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
            "images":{"a":{"file":"a.png"},"d":{"file":"d.png","color_space":"Non-Color"}},
            "materials":materials,
            "programs":{"x":{"source":""}},
            "animation":[{"tracks":[{"path":["materials","paint","inputs","base_color"],"keys":[[0,[0,1,0]]]}]}]}))
        .unwrap()
    }
    fn values(scene: &Scene, name: &str) -> Result<OpenPbrRecord> {
        let mut words = std::collections::BTreeMap::new();
        openpbr_values(
            scene,
            Some(name),
            &animated_inputs(scene)
                .unwrap()
                .remove(name)
                .unwrap_or_default(),
            |image| {
                let next = words.len() as u64 + 7;
                Ok(*words.entry(image.to_owned()).or_insert(next))
            },
        )
    }

    #[test]
    fn receiver_queries_require_possible_solid_transmission() {
        let material = |thin: bool, program: bool, animated: bool, mapped: bool, transmission: bool| {
            let mut pending = PendingMaterial::program(u32::from(program), false);
            let mut builder = RecordBuilder::openpbr();
            builder.set("geometry_thin_walled", &[f32::from(thin)]);
            pending.record.values = builder.record.values;
            pending.transmissive = transmission;
            if animated {pending.unique = Some("animated".into());}
            if mapped {
                let input = param("geometry_thin_walled").0;
                pending.record.inputs[input] |= 1;
            }
            pending.finish()
        };
        // Variant gates must agree with the planner, including tracks, programs
        // and mapped wall type. A false gate removes the entire transport path.
        for thin in [false, true] {
            for program in [false, true] {
                for animated in [false, true] {
                    for mapped in [false, true] {
                        for transmission in [false, true] {
                            let m = material(thin, program, animated, mapped, transmission);
                            assert_eq!(m.features & FEATURE_TRANSMISSION != 0, transmission);
                            assert_eq!(m.features & FEATURE_SOLID_TRANSMISSION != 0, m.solid_transmission);
                        }
                    }
                }
            }
        }
        assert!(material(false, false, false, false, true).solid_transmission);
        assert!(!material(true, false, false, false, true).solid_transmission);
        assert!(material(true, true, false, false, true).solid_transmission);
        assert!(material(true, false, true, false, true).solid_transmission);
        assert!(material(true, false, false, true, true).solid_transmission);
        assert!(!material(false, true, true, true, false).solid_transmission);
    }
    #[test]
    fn solid_shadow_absorption_keeps_unknowns_but_omits_clear_interfaces() {
        let material=|depth:f32,color:[f32;3],thin:bool,program:bool,animated:bool,mapped:Option<&str>| {
            let mut pending=PendingMaterial::program(u32::from(program),false);
            let mut builder=RecordBuilder::openpbr();builder.set("transmission_depth",&[depth]);builder.set("transmission_color",&color);builder.set("geometry_thin_walled",&[f32::from(thin)]);
            pending.record.values=builder.record.values;pending.transmissive=true;
            if animated {pending.unique=Some("animated".into());}
            if let Some(field)=mapped {pending.record.inputs[param(field).0]|=1;}
            pending.finish().solid_absorption
        };
        assert!(!material(0.,[0.5;3],false,false,false,None));
        assert!(!material(1.,[1.;3],false,false,false,None));
        assert!(material(1.,[0.5,1.,1.],false,false,false,None));
        assert!(!material(1.,[0.5;3],true,false,false,None));
        assert!(material(0.,[1.;3],false,true,false,None));
        assert!(material(0.,[1.;3],false,false,true,None));
        assert!(material(0.,[0.5;3],false,false,false,Some("transmission_depth")));
        assert!(material(1.,[1.;3],false,false,false,Some("transmission_color")));
    }

    #[test]
    fn subsurface_accepts_dithered_and_blended_coverage() {
        let mut s=scene(serde_json::json!({
            "coverage":{"inputs":{"geometry_opacity":0.5,"subsurface_weight":0.5}},
            "transmittance":{"opacity":"transmittance","inputs":{"geometry_opacity":0.5,"subsurface_weight":0.5}}
        }));
        s.animation.clear();
        assert!(values(&s,"coverage").is_ok());
        assert!(values(&s,"transmittance").is_ok());
    }
    #[test]
    fn transmission_preserves_subsurface_substrate() {
        let mut s=scene(serde_json::json!({"mixed":{"inputs":{
            "transmission_weight":0.5,"subsurface_weight":0.5
        }}}));
        s.animation.clear();
        assert!(values(&s,"mixed").unwrap().capabilities.subsurface);
    }
    #[test]
    fn shading_features_preserve_metal_tracks_textures_and_programs() {
        let s = scene(serde_json::json!({
            "paint": {},
            "coat": {"inputs": {"coat_weight": 0.5}},
            "metal": {"inputs": {"base_metalness": 0.5}},
            "both": {"inputs": {"base_metalness": 1, "fuzz_weight": 0.5}},
            "texture": {"inputs": {"base_metalness": {"image": "d"}}}
        }));
        let finish = |v: OpenPbrRecord| {
            let mut pending = PendingMaterial::program(0, false);
            pending.record = v.builder.record;
            pending.shading = v.shading;
            pending.finish().features as u8 & (SHADING_LAYERS | SHADING_METAL)
        };
        for (name, expected) in [
            ("paint", 0),
            ("coat", SHADING_LAYERS),
            ("metal", SHADING_METAL),
            ("both", SHADING_LAYERS | SHADING_METAL),
            ("texture", SHADING_METAL),
        ] {
            assert_eq!(finish(values(&s, name).unwrap()), expected, "{name}");
        }
        // A zero initial weight can become metallic on a later frame/instance phase.
        let animated = ["base_metalness".to_owned()].into_iter().collect();
        let tracked = openpbr_values(&s, Some("paint"), &animated, |_| Ok(0)).unwrap();
        assert_eq!(finish(tracked), SHADING_METAL);
        assert_eq!(
            PendingMaterial::program(1, false).finish().features as u8
                & (SHADING_LAYERS | SHADING_METAL),
            SHADING_LAYERS | SHADING_METAL
        );
    }
    #[test]
    fn specialization_keeps_dynamic_and_program_capabilities() {
        let s = scene(serde_json::json!({
            "paint": {},
            "coat": {"inputs": {"coat_weight": 0.5}},
            "normal": {"inputs": {"geometry_normal": {"image":"d"}}},
            "coat_normal": {"inputs": {"coat_weight":1,"geometry_coat_normal": {"image":"d"}}},
            "tinted": {"inputs": {"specular_color": [0.4, 0.6, 0.8]}}
        }));
        let finish = |name| {
            let v = values(&s, name).unwrap();
            let mut p = PendingMaterial::program(0, true);
            p.record = v.builder.record;
            p.shading = v.shading;
            p.finish().features
        };
        assert_eq!(finish("paint"), FEATURE_SPECIALIZED);
        assert_ne!(finish("coat") & FEATURE_COAT, 0);
        assert_eq!(finish("coat") & FEATURE_FUZZ, 0);
        assert_ne!(
            finish("normal") & (1 << (param("geometry_normal").0 + 2)),
            0
        );
        assert_ne!(finish("tinted") & FEATURE_TINT, 0);
        assert_ne!(finish("coat_normal") & texture_feature("geometry_coat_normal"), 0);
        assert_ne!(finish("coat_normal") & FEATURE_COAT, 0);
        let mut program = PendingMaterial::program(1, false);
        program.integrate_opacity(Some(vork_scene::Opacity::Coverage),false,false);
        let program = program.finish().features;
        let required =
            FEATURE_COAT | FEATURE_FUZZ | FEATURE_TINT | FEATURE_MASK | FEATURE_SIDED | 3;
        assert_eq!(program & required, required);
        assert_eq!(program & FEATURE_LOBES, 0);
        assert_eq!(program & (FEATURE_ANISOTROPY | FEATURE_FILM), FEATURE_ANISOTROPY | FEATURE_FILM);
        let mut hair = PendingMaterial::program(1, false);
        hair.hair = true;
        assert_eq!(hair.finish().features & (FEATURE_ANISOTROPY | FEATURE_FILM), 0);
        let mut mixed = PendingMaterial::program(1, false);
        mixed.lobes = true;
        assert_ne!(mixed.finish().features & FEATURE_LOBES, 0);
        let mut tracked = PendingMaterial::program(0, true);
        tracked.shading = SHADING_LAYERS | SHADING_METAL;
        tracked.unique = Some("paint".into());
        assert_eq!(
            tracked.finish().features & required,
            required & !(FEATURE_MASK | FEATURE_SIDED)
        );
    }

    #[test]
    fn scene_variant_budget_merges_only_drawn_materials_conservatively() {
        let mut materials: Vec<_> = (0..=constants::SCENE_SHADING_VARIANTS)
            .map(|i| {
                let mut m = PendingMaterial::program(0, true).finish();
                m.features |= 1 << (i + 2);
                m
            })
            .collect();
        let original: Vec<_> = materials.iter().map(|m| m.features).collect();
        let layout = crate::geometry::VertexLayout::for_vertices(&[Vertex::zeroed()], false);
        bound_features(
            &mut materials,
            (0..constants::SCENE_SHADING_VARIANTS).map(|i| (layout, i)),
        );
        assert_eq!(
            materials.iter().map(|m| m.features).collect::<Vec<_>>(),
            original,
            "unused material does not consume budget"
        );
        let len = materials.len();
        bound_features(&mut materials, (0..len).map(|i| (layout, i)));
        let union = original.iter().fold(0, |a, b| a | b);
        assert!(materials.iter().all(|m| m.features == union));
        for (m, before) in materials.iter().zip(original) {
            assert_eq!(m.features & before, before, "no capability lost");
        }
    }

    #[test]
    fn scene_variant_budget_counts_layout_multiplication() {
        let layout = crate::geometry::VertexLayout::for_vertices(&[Vertex::zeroed()], false);
        let mut other = layout;
        other.deform = !other.deform;
        let mut materials: Vec<_> = (0..constants::SCENE_SHADING_VARIANTS)
            .map(|i| {
                let mut m = PendingMaterial::program(0, true).finish();
                m.features |= 1 << (i + 2);
                m
            })
            .collect();
        let len = materials.len();
        bound_features(
            &mut materials,
            [layout, other]
                .into_iter()
                .flat_map(|l| (0..len).map(move |i| (l, i))),
        );
        assert!(
            materials
                .iter()
                .all(|m| m.features == materials[0].features)
        );
    }

    #[test]
    fn too_many_mandatory_layouts_use_generic_shading() {
        let base = crate::geometry::VertexLayout::for_vertices(&[Vertex::zeroed()], false);
        let uv_sets = [
            base.uv,
            [0, constants::VERTEX_DEFAULT_UV],
            [constants::VERTEX_DEFAULT_UV, 0],
            [0, 0],
            [0, 1],
        ];
        let layouts: Vec<_> = uv_sets
            .into_iter()
            .flat_map(|uv| {
                (0..6).map(move |flags| crate::geometry::VertexLayout {
                    deform: flags >= 4,
                    tangent: flags >= 4 || flags & 2 != 0,
                    color: flags & 1 != 0,
                    uv,
                    ..base
                })
            })
            .take(constants::SCENE_SHADING_VARIANTS + 1)
            .collect();
        let mut masked = PendingMaterial::program(0, false);
        masked.integrate_opacity(Some(vork_scene::Opacity::Coverage),false,false);
        let mut materials = [masked.finish()];
        bound_features(&mut materials, layouts.into_iter().map(|l| (l, 0)));
        assert_eq!(
            materials[0].features,
            FeatureMask::from(SHADING_LAYERS | SHADING_METAL),
            "generic shading retains both dynamic lobes and every default texture/light path"
        );
    }

    #[test]
    fn spatial_subsurface_reserves_animated_thin_walled_state() {
        let scene: vork_scene::v2::Scene=vork_scene::v2::from_test_value(serde_json::json!({"version":2,"nodes":[],
            "materials":{"leaf":{"inputs":{"subsurface_weight":1,"geometry_thin_walled":true}}}})).unwrap();
        for tracked in [false,true] {
            let dynamic=if tracked {["geometry_thin_walled".to_owned()].into_iter().collect()} else {Default::default()};
            let record=openpbr_values(&scene,Some("leaf"),&dynamic,|_|Ok(0)).unwrap();
            assert!(record.capabilities.subsurface);
            assert_eq!(record.spatial_subsurface(),tracked);
        }
    }

    #[test]
    fn subsurface_capability_reserves_animated_zero_weight() {
        let scene=scene(serde_json::json!({
            "plain":{}, "skin":{"inputs":{"subsurface_weight":0.5}}
        }));
        for (name, tracked, expected) in [("plain",false,false),("plain",true,true),("skin",false,true)] {
            let animated=if tracked {["subsurface_weight".to_owned()].into_iter().collect()} else {Default::default()};
            let values=openpbr_values(&scene,Some(name),&animated,|_|Ok(0)).unwrap();
            assert_eq!(values.capabilities.subsurface,expected);

        }
    }

    #[test]
    fn surface_response_capabilities_follow_the_authored_inputs() {
        let mut program=PendingMaterial::program(1,false);
        assert_eq!(program.clone().finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),FEATURE_ANISOTROPY|FEATURE_FILM);
        program.response_program=0;
        assert_eq!(program.clone().finish().features & (FEATURE_COAT|FEATURE_FUZZ|FEATURE_TINT),0);
        for feature in [FEATURE_COAT,FEATURE_FUZZ,FEATURE_TINT] {
            program.response_program=feature;
            assert_eq!(program.clone().finish().features & (FEATURE_COAT|FEATURE_FUZZ|FEATURE_TINT),feature);
        }
        program.response_program=0;
        assert_eq!(program.clone().finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),0);
        program.response_program=FEATURE_FILM;
        assert_eq!(program.finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),FEATURE_FILM);
        let plain=PendingMaterial::program(0,false);
        assert_eq!(plain.clone().finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),0);
        let mut color_track=plain.clone();color_track.unique=Some("paint".into());
        assert_eq!(color_track.finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),0);
        let mut anisotropy_track=plain.clone();anisotropy_track.response_animated=FEATURE_ANISOTROPY;
        assert_eq!(anisotropy_track.finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),FEATURE_ANISOTROPY);
        let mut film_track=plain;film_track.response_animated=FEATURE_FILM;
        assert_eq!(film_track.finish().features & (FEATURE_ANISOTROPY|FEATURE_FILM),FEATURE_FILM);
        for (input,capability) in [("specular_roughness_anisotropy",FEATURE_ANISOTROPY),("thin_film_weight",FEATURE_FILM)] {
            let mut pending=PendingMaterial::program(0,false);
            pending.record.values[value_word(param(input).0)]=0.5;
            assert_ne!(pending.finish().features&capability,0);
        }
    }

    #[test]
    fn openpbr_records_reject_what_is_not_applied() {
        for (inputs, why) in [
            (
                serde_json::json!({"specular_roughness":"a"}),
                "sRGB data image",
            ),
            (
                serde_json::json!({"coat_roughness_anisotropy":0.5}),
                "parameter vork does not shade",
            ),
            (
                serde_json::json!({"geometry_thin_walled":{"image":"d"}}),
                "boolean texture",
            ),
        ] {
            let s = scene(serde_json::json!({"paint":{"inputs":inputs}}));
            assert!(values(&s, "paint").is_err(), "accepted {why}");
        }
    }
    #[test]
    fn scalar_texture_diagnostic_prescribes_a_valid_colour_space() {
        let mut s = scene(serde_json::json!({"paint":{"inputs":{"specular_roughness":{"image":"a"}}}}));
        let error = values(&s, "paint").err().expect("sRGB scalar texture must fail");
        assert!(error.to_string().contains("color_space \"Non-Color\""), "{error}");
        let mut authored = serde_json::to_value(&s).unwrap();
        authored["images"]["a"]["color_space"] = "Non-Color".into();
        s = vork_scene::v2::from_test_value(authored).unwrap();
        values(&s, "paint").unwrap();
    }
    /// Only dithered coverage selects the depth and control entries that read
    /// SV_PrimitiveID; opaque, cutout and blended table materials never do.
    #[test]
    fn only_dithered_materials_select_hashed_coverage_entries() {
        use crate::{bundles::Kind, forward_variant::FEATURE_DITHERED, pipelines::Role};
        use vork_scene::Opacity::{Transmittance, Coverage};
        for (method, transmission, dithered) in [
            (None, false, false),
            (Some(Transmittance), false, false),
            (Some(Coverage), false, true),
            (Some(Coverage), true, true),
            (None, true, false),
        ] {
            let mut pending = PendingMaterial::program(0, false);
            if method.is_some() || transmission {
                pending.integrate_opacity(method, transmission, false);
            }
            let material = pending.finish();
            assert_eq!(material.dithered, dithered, "{method:?} {transmission}");
            assert_eq!(material.features & FEATURE_DITHERED, 0, "a pipeline-role bit, not a material capability");
            let flags = crate::plan::DrawFlags { hair: false, transparent: material.transparent, moving: false,
                casts_shadows: true, alpha_mask: material.alpha_mask };
            let mut controls = 0;
            for &kind in Role::kinds(&flags, true) {
                match Role::of(kind, &flags, &material) {
                    Role::Controls(m) | Role::TransparentControls(m) => {
                        controls += 1;
                        assert_eq!(m & FEATURE_DITHERED != 0, dithered, "{method:?} {transmission}");
                    }
                    Role::MaskedDepth { hashed } => assert_eq!(hashed, dithered),
                    _ => (),
                }
            }
            assert_eq!(controls, 1);
            if material.alpha_mask && !material.transparent {
                assert_eq!(Role::of(Kind::Depth, &flags, &material), Role::MaskedDepth { hashed: dithered });
            }
        }
    }

    #[test]
    fn dithered_programs_cast_coverage_shadows() {
        let mut pending=PendingMaterial::program(1,false);
        pending.integrate_opacity(Some(vork_scene::Opacity::Coverage),false,true);
        let material=pending.finish();
        assert!(material.alpha_mask,"camera dither still needs evaluated coverage");
        assert!(material.shadow_coverage);
        assert!(material.shadow_animated);

    }

    /// Zero emission factors (an authored black colour or zero luminance, a texture
    /// times either) are not emitters; nonzero constants and emission tracks are.
    #[test]
    fn provably_dark_emission_is_not_an_emitter() {
        let pending = |record, emission_animated| {
            let mut p = PendingMaterial::program(0, false);
            p.record = record;
            p.emission_animated = emission_animated;
            p.finish()
        };
        let s: Scene = vork_scene::v2::from_test_value(serde_json::json!({"version":2,
            "nodes":[{"type":"camera","material":{"program":"std/camera","inputs":{"projection":"persp"}}}],
            "images":{"a":{"file":"a.png"}},
            "materials":{
                "plain":{},
                "black":{"inputs":{"emission_color":[0,0,0],"emission_luminance":1}},
                "off_map":{"inputs":{"emission_color":{"image":"a"},"emission_luminance":0}},
                "lit_map":{"inputs":{"emission_color":{"image":"a"},"emission_luminance":1}},
                "glow":{"inputs":{"emission_luminance":1}}}}))
        .unwrap();
        for (name, emits) in [("plain", false), ("black", false), ("off_map", false),
            ("lit_map", true), ("glow", true)] {
            let record = values(&s, name).unwrap().builder.record;
            let m = pending(record, false);
            assert_eq!(m.source_emission, emits, "{name}");
            let tracked = pending(record, true);
            assert!(tracked.source_emission, "{name} with an emission track");
        }
    }

    #[test]
    fn animated_records_never_merge_with_identical_static_ones() {
        let s = scene(serde_json::json!({"paint":{}}));
        let record = values(&s, "paint").unwrap().builder.record;
        let pending = |unique: Option<&str>| PendingMaterial {
            uv_sources: [0, 1],
            emission_animated: false,
            source_emission: false,
            record,
            taps: Vec::new(),
            alpha_mask: false,
            double_sided: false,
            transparent: false,
            transmissive: false,
            lobes: false,
            lit: false,
            hair: false,
            physical_hair: false,
            opacity_animated: false,
            shading: 0,
            response_animated: 0,
            response_program: 0,
            unique: unique.map(str::to_owned),
        };
        assert_ne!(pending(None).key(), pending(Some("paint")).key());
    }
    #[test]
    fn material_tracks_read_the_installed_record() {
        let s =
            scene(serde_json::json!({"paint":{"inputs":{"base_color":[1.,1.,1.]}},"p":{"program":"x"}}));
        let mut records = vec![MaterialRecord::zeroed(); 8];
        records[3] = values(&s, "paint").unwrap().builder.record;
        records[7] = records[3];
        let bytes = bytemuck::cast_slice::<_, u8>(&records);
        let base = OPENPBR.iter().position(|p| p.name == "base_color").unwrap();
        let targets = track_targets(("paint", "base_color"), &[3, 7], bytes).unwrap();
        assert_eq!(
            targets,
            vec![(3, base, vec![1., 1., 1.]), (7, base, vec![1., 1., 1.])]
        );
        let roughness = track_targets(("paint", "specular_roughness"), &[3], bytes).unwrap();
        assert_eq!(roughness[0].2, vec![0.3]);
        assert!(track_targets(("paint", "thin_film_weight"), &[3], bytes).is_ok());
        assert!(track_targets(("p", "anything"), &[3], bytes).is_err());
        assert!(track_targets(("paint", "base_color"), &[9], bytes).is_err());
        assert_eq!(
            animated_inputs(&s).unwrap()["paint"]
                .iter()
                .collect::<Vec<_>>(),
            ["base_color"]
        );
    }
}

#[cfg(test)]
#[test]
fn background_image_decode_preserves_pixels_cache_and_warm_laziness() -> Result<()> {
    let root = std::env::temp_dir().join(format!("vork-background-images-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let store = disk_cache::Store::for_test(root.join("cache"));
    let mut expected = Vec::new();
    let mut images = Vec::new();
    for index in 0..12 {
        let image = if index % 2 == 0 {
            image::DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(
                19,
                7,
                image::Rgb([17 + index, 32768, 65000]),
            ))
        } else {
            image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
                19,
                7,
                image::Rgba([17 + index as u8, 42, 93, 128]),
            ))
        };
        let name = format!("{index}.png");
        image.save(root.join(&name))?;
        images.push(serde_json::json!({"uri": name}));
        expected.push((
            image.width(),
            image.height(),
            image_channel_format(image.color())?,
            image.into_bytes(),
        ));
    }
    let path = root.join("source.gltf");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "asset": {"version": "2.0"}, "images": images,
        }))?,
    )?;
    let mut decoders = ImageDecoders::default();
    let ((_, _, images), keys) = load_cached(&path, &store, &mut decoders, crate::fuzz::MAX_SIDE)?;
    // Request one image; leave the others unused, as an imported material can.
    assert_eq!(images[3].data()?.pixels, expected[3].3);
    drop(decoders); // Drop must join unused work and finish its cache publication.
    for (index, key) in keys.into_iter().enumerate() {
        let data = read_decoded(&store.path("decoded-texture", 3, key))
            .context("unfinished decoded cache")?;
        assert_eq!(
            (data.width, data.height, data.format),
            (expected[index].0, expected[index].1, expected[index].2)
        );
        assert_eq!(data.pixels, expected[index].3);
    }
    drop(images);
    let mut decoders = ImageDecoders::default();
    let ((_, _, images), _) = load_cached(&path, &store, &mut decoders, crate::fuzz::MAX_SIDE)?;
    decoders.finish()?;
    assert!(
        images
            .iter()
            .all(|image| image.pending.is_none() && image.data.get().is_none())
    );
    drop(images);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[cfg(test)]
#[test]
fn background_image_decode_reports_unused_source_errors_at_install_barrier() -> Result<()> {
    let root =
        std::env::temp_dir().join(format!("vork-background-image-error-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let store = disk_cache::Store::for_test(root.join("cache"));
    let source = root.join("broken.png");
    image::save_buffer(&source, &[17; 32 * 32 * 4], 32, 32, image::ColorType::Rgba8)?;
    let mut encoded = std::fs::read(&source)?;
    let idat = encoded
        .windows(4)
        .position(|bytes| bytes == b"IDAT")
        .unwrap();
    encoded[idat + 6] ^= 255;
    let png = gltf::image::Source::Uri { uri: "broken.png", mime_type: None };
    assert!(image_metadata(&png, &encoded, crate::fuzz::MAX_SIDE).is_ok());
    std::fs::write(&source, encoded)?;
    let path = root.join("unused.gltf");
    std::fs::write(
        &path,
        br#"{"asset":{"version":"2.0"},"images":[{"uri":"broken.png"}]}"#,
    )?;
    let mut decoders = ImageDecoders::default();
    let ((_, _, images), _) = load_cached(&path, &store, &mut decoders, crate::fuzz::MAX_SIDE)?;
    assert!(images[0].data.get().is_none()); // No material ever asks for these pixels.
    let error = decoders.finish().unwrap_err().to_string();
    assert!(
        error.contains("unused.gltf") && error.contains("image 0"),
        "{error}"
    );
    drop(images);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn unsupported_gltf_extensions_are_summarised_per_document() {
    let document = gltf::Gltf::from_slice(br#"{
        "asset":{"version":"2.0"},
        "materials":[
            {"extensions":{"KHR_materials_iridescence":{}}},
            {"extensions":{"KHR_materials_iridescence":{}}}
        ],
        "buffers":[{"byteLength":36,"uri":"positions.bin"}],
        "bufferViews":[{"buffer":0,"byteLength":36}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
        "meshes":[{"primitives":[
            {"attributes":{"POSITION":0},"extensions":{"KHR_materials_variants":{}}},
            {"attributes":{"POSITION":0},"extensions":{"KHR_materials_variants":{}}}
        ]}]
    }"#).unwrap().document;
    let warnings = gltf_extension_warnings(&document);
    assert_eq!(warnings, [
        "KHR_materials_iridescence: not applied yet (2 materials, 0 primitives affected)",
        "KHR_materials_variants: not applied yet; using primitive materials (0 materials, 2 primitives affected)",
    ]);
    let supported = gltf::Gltf::from_slice(br#"{"asset":{"version":"2.0"},
        "materials":[{"extensions":{"KHR_materials_clearcoat":{"clearcoatFactor":1}}}]}
    "#).unwrap().document;
    assert!(gltf_extension_warnings(&supported).is_empty());
    let coat_normals = gltf::Gltf::from_slice(br#"{"asset":{"version":"2.0"},
        "materials":[
            {"extensions":{"KHR_materials_clearcoat":{"clearcoatNormalTexture":{"index":0}}}},
            {"extensions":{"KHR_materials_clearcoat":{"clearcoatNormalTexture":{"index":0}}}}
        ]}
    "#).unwrap().document;
    assert_eq!(gltf_extension_warnings(&coat_normals), [
        "KHR_materials_clearcoat: clearcoatNormalTexture is not applied yet (coat normals are not shaded) (2 materials, 0 primitives affected)"
    ]);
}

#[cfg(test)]
mod source_capacity_tests {
    use super::*;
    #[test]
    fn every_input_can_bind_a_distinct_full_width_source() -> Result<()> {
        let mut builder=RecordBuilder::openpbr();
        for input in 0..=OCCLUSION_INPUT {
            builder.bind_index(input,Tap::plain((1u64<<40)+input as u64),4)?;
            assert_eq!(builder.record.inputs[input]&constants::MATERIAL_TAP_INDEX_MASK,input as u32+1);
            assert_eq!(builder.record.inputs[input]>>28,4);
            assert_eq!(builder.taps[input][8],256);
        }
        assert!(builder.taps.len()>15);
        Ok(())
    }
}

#[cfg(test)]
mod descriptor_capacity_tests {
    use super::*;
    #[test]
    fn material_tables_sample_beyond_1024_textures_and_64_samplers() -> Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(());};
        let images=Default::default();
        let mut installer=TextureInstaller::new(&r,&images);
        let samplers:Vec<_>=(0..65).map(|i|r.device.create_sampler(&wgpu::SamplerDescriptor {lod_min_clamp:i as f32/128.,lod_max_clamp:32.,..Default::default()})).collect();
        for i in 0..1025 {
            let texture=r.device.create_texture(&wgpu::TextureDescriptor {label:Some("descriptor capacity fixture"),size:extent(1,1),mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
            r.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},&[23,191,91,255],wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},extent(1,1));
            installer.slot(texture.create_view(&Default::default()),samplers[i%65].clone());
        }
        assert_eq!(installer.counts()?,(1025,65));
        let material_layout=r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:None,entries:&crate::pipelines::material_entries_for(1025,65)});
        let table=installer.table(&material_layout,vec![0;16])?;
        let output_layout=r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:None,entries:&[crate::pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})]});
        let output=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:16,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:16,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&output_layout,entries:&[entry(0,output.as_entire_binding())]});
        let layout=r.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:None,bind_group_layouts:&[Some(&output_layout),Some(&material_layout)],immediate_size:0});
        let source="import vork_material_table; [[vk::binding(0,0)]] RWStructuredBuffer<float4> output : register(u0); [shader(\"compute\")][numthreads(1,1,1)] void capacity_main() {output[0]=material_image(uint2(1024,64),float2(.5),float2(0),float2(0));}";
        let mut modules=crate::native::SOURCES.to_vec();modules.push(("descriptor_capacity",source));
        let metal=r.adapter_info.backend==wgpu::Backend::Metal;
        let (_,target,flags)=if metal {crate::forward_variant::METAL_TARGET} else {crate::forward_variant::SPIRV_TARGET};
        let registers=crate::binding_abi::material_registers(1);
        let args:Vec<_>=["-target",target].into_iter().chain(flags.iter().copied()).chain(registers.iter().map(String::as_str)).collect();
        let code=vork_slang::Compiler::discover().compile_modules(&modules,"descriptor_capacity",&[("capacity_main","compute")],&args)?;
        let shader=if metal {crate::native::module(&r.device,"descriptor capacity",None,Some(&String::from_utf8(code)?),(1,1,1))}
            else {crate::native::module(&r.device,"descriptor capacity",Some(&code),None,(1,1,1))};
        let pipeline=r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:None,layout:Some(&layout),module:&shader,entry_point:Some("capacity_main"),compilation_options:Default::default(),cache:None});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.set_bind_group(1,table.bind(),&[]);pass.dispatch_workgroups(1,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,16);
        let submission=crate::command_finish::Segments::submit(&r.queue,encoder);
        let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |value|{let _=tx.send(value);});
        r.wait(Some(submission))?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);
        let color:&[f32]=bytemuck::cast_slice(&mapped);
        for (actual,expected) in color.iter().zip([23.,191.,91.,255.]) {assert!((actual-expected/255.).abs()<1e-5);}
        Ok(())
    }
}
