//! Install-time image sources. Render names resolve to slots in the material table.
use std::path::PathBuf;

pub(crate) fn missing_image(path: &std::path::Path) -> image::DynamicImage {
    crate::warnings::dropped("missing_image", serde_json::json!({"file":path}),
        "using a 1x1 magenta texture; restore the image or correct its path.".into());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 255, 255])))
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) enum TextureSource {
    File(PathBuf),
    Render(String),
    Media { name: String, file: PathBuf, time: Option<vork_time::Clock> },
}
impl TextureSource {
    /// Render images are rendered again every frame; file images never change.
    pub(crate) fn changes_between_frames(&self) -> bool { matches!(self, Self::Render(_) | Self::Media {..}) }
    pub(crate) fn scene(name: &str, image: &vork_scene::v2::Image) -> Self {
        match &image.render {
            Some(_) => Self::Render(name.into()),
            None => {
                let file = PathBuf::from(image.file.as_ref().expect("validated file image"));
                if crate::footage::is_media(&file) {Self::Media {name:name.into(),file,time:image.time.clone()}}
                else {Self::File(file)}
            },
        }
    }
}

/// Decode an image file at its native precision: 8/16-bit integer or 32-bit float
/// channels (PNG, JPEG, TIFF, Radiance HDR, OpenEXR), or IES photometry. Material,
/// program and environment images share this one decoder.
pub(crate) fn decode(
    path: &std::path::Path,
    bytes: &[u8],
    max_side: u32,
) -> anyhow::Result<image::DynamicImage> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ies")) {
        return crate::ies::decode(bytes);
    }
    decode_format(image::ImageFormat::from_path(path)?, bytes, max_side)
}

/// The raster image decoder for scene and glTF images. Each image uploads as one
/// texture, so a side beyond the device's `max_side` is refused from the header,
/// before any pixel is decoded. Upstream decoders panic on some malformed files;
/// that is returned as an error.
pub(crate) fn decode_format(
    format: image::ImageFormat,
    bytes: &[u8],
    max_side: u32,
) -> anyhow::Result<image::DynamicImage> {
    crate::failure::contain("the image decoder", || {
        let reader = bounded(format, bytes, max_side)?;
        Ok(reader.decode()?)
    })
}

/// Size and channels from the header alone (glTF images whose pixels are cached),
/// with the same format and bounds as `decode_format`.
pub(crate) fn header(
    format: image::ImageFormat,
    bytes: &[u8],
    max_side: u32,
) -> anyhow::Result<(u32, u32, image::ColorType)> {
    use image::ImageDecoder;
    crate::failure::contain("the image decoder", || {
        let decoder = bounded(format, bytes, max_side)?.into_decoder()?;
        let (width, height) = decoder.dimensions();
        Ok((width, height, decoder.color_type()))
    })
}

/// A reader for an image whose sides, and for EXR each part's data window, fit
/// `max_side`.
fn bounded(
    format: image::ImageFormat,
    bytes: &[u8],
    max_side: u32,
) -> anyhow::Result<image::ImageReader<std::io::Cursor<&[u8]>>> {
    let side = |what: &str, width: usize, height: usize| {
        anyhow::ensure!(
            width.max(height) <= max_side as usize,
            "{}", crate::gpu_memory::limit_message(format_args!("the {what} is {width}x{height}"), format_args!("{width}x{height} texels"), format_args!("{max_side} texels per texture side"), "reduce the source image resolution")
        );
        Ok(())
    };
    if format == image::ImageFormat::OpenExr {
        // The EXR decoder allocates each part's data window, which a header may
        // place far outside the image it displays.
        for header in &exr::meta::MetaData::read_from_buffered(bytes, false)?.headers {
            side("data window", header.layer_size.width(), header.layer_size.height())?;
        }
    }
    let reader = || image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = reader().into_dimensions()?;
    side("image", width as usize, height as usize)?;
    Ok(reader())
}

/// IEC 61966-2-1 decoding, shared by raw and compressed image installation.
pub(crate) fn decode_srgb(value: f32) -> f32 {
    let curve = crate::constants::SRGB_DECODE_CURVE;
    if value <= curve[0] {
        value / curve[1]
    } else {
        ((value + curve[2]) / curve[3]).powf(curve[4])
    }
}

#[cfg(test)]
mod tests {
    /// QA fuzz reproducers in crates/vork-fuzz/seeds/image (first byte: the fuzz
    /// harness's format selector). Each aborted, panicked, took gigabytes or ran for
    /// tens of seconds; each is now refused with its cause or returns its panic.
    #[test]
    #[cfg(vork_private_tests)]
    fn malformed_images_are_refused_before_decoding_or_contained() {
        let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vork-fuzz/seeds/image");
        let mut cases = vec![
            ("exr-dwa-ac-count-abort", "exr", "the image is 7x65537"),
            ("exr-dwa-capacity-overflow", "exr", "the image is 7x65537"),
            ("exr-dwaa-truncated-slow", "exr", "the data window is 83886082x66"),
            ("exr-data-window-5gb", "exr", "the data window is 352321538x2"),
            ("exr-dwa-ac-count-small-window", "exr", "DWA section sizes exceed the chunk"),
            // zune-jpeg 0.5.15 asserts in its AVX2 upsampler on this one.
            ("jpeg-avx2-upsampler-assert", "jpg", "the image is 4x16388"),
        ];
        // zune-jpeg 0.5.15 unwraps None in its AVX2 IDCT, the path it takes on
        // x86-64 hosts with AVX2 (every CI and VM host).
        if cfg!(target_arch = "x86_64") {
            cases.push(("jpeg-idct-unwrap", "jpg", "the image decoder failed on malformed input (upstream panic at zune-jpeg-0.5.15/src/idct/avx2.rs:"));
        }
        for (seed, extension, cause) in cases {
            let bytes = std::fs::read(seeds.join(seed)).unwrap();
            let path = std::path::PathBuf::from(format!("image.{extension}"));
            let error = crate::fuzz::image(&path, &bytes[1..]).expect_err(seed);
            assert!(format!("{error:#}").contains(cause), "{seed}: {error:#}");
        }
    }
}
