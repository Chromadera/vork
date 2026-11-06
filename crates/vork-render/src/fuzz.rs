//! CPU-only parser entry points for `crates/vork-fuzz`; not renderer API.
//! Each runs the decoding a scene install performs on user files, without a device.
use anyhow::{Context, Result, ensure};
use std::path::Path;

/// LM-63 photometry, as exact emitters and IES image sources decode it.
pub fn ies(bytes: &[u8]) -> Result<()> {
    crate::ies::parse(bytes)?.words();
    crate::ies::decode(bytes).map(drop)
}

/// The texture side images are bounded by without a device: llvmpipe's and Apple
/// GPUs' 2D limit.
pub(crate) const MAX_SIDE: u32 = 16384;

/// A scene image file as material installation decodes and converts it; the
/// path supplies only the extension that selects the decoder.
pub fn image(path: &Path, bytes: &[u8]) -> Result<()> {
    let data =
        crate::materials::decoded_data(crate::texture_source::decode(path, bytes, MAX_SIDE)?)?;
    if !crate::materials::eight_bit(data.format) {
        crate::materials::half_texels(&data, true)?;
    }
    Ok(())
}

/// A scene buffer file: numeric attributes, or a raw NanoVDB grid. Other grid
/// files would run the external loader process, so they are rejected here.
pub fn buffer(buffer: &vork_scene::v2::Buffer, base: &Path) -> Result<()> {
    if buffer.grid.is_none() {
        return crate::buffers::AttributeData::decode(buffer, base).map(drop);
    }
    let path = base.join(buffer.file.as_deref().context("grid file required")?);
    let bytes = std::fs::read(path)?;
    ensure!(
        bytes.len() >= 736 && matches!(&bytes[..8], b"NanoVDB0" | b"NanoVDB1"),
        "fuzzing reads raw NanoVDB grids only"
    );
    crate::vdb::Grid::load(buffer, base).map(drop)
}

/// Ear clipping of polygon cages.
pub fn polygon(positions: &[[f32; 3]], indices: &[u32], sizes: &[u32]) -> Result<()> {
    crate::polygon::triangulate(positions, indices, sizes).map(drop)
}

fn consume<T>(items: impl IntoIterator<Item=T>) {
    for item in items {std::hint::black_box(item);}
}

/// glTF document compatibility, buffers and embedded images as installation reads
/// them (`materials::load_cached`); external URIs are refused rather than read.
pub fn gltf(path: &Path) -> Result<()> {
    let gltf::Gltf { document, blob } = crate::import::open(path)?;
    let buffers = crate::import::buffers(&document, None, blob)?;
    for image in document.images() {
        let source = image.source();
        let encoded = crate::materials::encoded_image(&image, &buffers, None)?.0;
        crate::materials::image_metadata(&source, &encoded, MAX_SIDE)?;
        crate::materials::decode_image_bytes(&source, &encoded, MAX_SIDE)?;
    }
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            reader.read_positions().map(consume);
            reader.read_normals().map(consume);
            reader.read_tangents().map(consume);
            reader.read_indices().map(|i| consume(i.into_u32()));
            for set in 0..2 {
                reader.read_tex_coords(set).map(|i| consume(i.into_f32()));
                reader.read_colors(set).map(|i| consume(i.into_rgba_f32()));
                reader.read_joints(set).map(|i| consume(i.into_u16()));
                reader.read_weights(set).map(|i| consume(i.into_f32()));
            }
            for (p, n, t) in reader.read_morph_targets() {
                p.map(consume);
                n.map(consume);
                t.map(consume);
            }
        }
    }
    for skin in document.skins() {
        let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
        reader.read_inverse_bind_matrices().map(consume);
    }
    for animation in document.animations() {
        for channel in animation.channels() {
            let reader = channel.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            reader.read_inputs().map(consume);
            reader.read_outputs().map(|o| match o {
                gltf::animation::util::ReadOutputs::Translations(i) => consume(i),
                gltf::animation::util::ReadOutputs::Rotations(i) => consume(i.into_f32()),
                gltf::animation::util::ReadOutputs::Scales(i) => consume(i),
                gltf::animation::util::ReadOutputs::MorphTargetWeights(i) => consume(i.into_f32()),
            });
        }
    }
    Ok(())
}
