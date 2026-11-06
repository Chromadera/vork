//! Blender ImageFormatSettings.exr_codec names (lowercased RNA enum tokens) and OpenEXR encodings.
use exr::prelude::{Blocks, Compression, Encoding, LineOrder};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExrCodec {
    None,
    Rle,
    Zips,
    #[default]
    Zip,
    Piz,
    Pxr24,
    B44,
    B44a,
    Dwaa,
    Dwab,
}
impl std::str::FromStr for ExrCodec {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "rle" => Ok(Self::Rle),
            "zips" => Ok(Self::Zips),
            "zip" => Ok(Self::Zip),
            "piz" => Ok(Self::Piz),
            "pxr24" => Ok(Self::Pxr24),
            "b44" => Ok(Self::B44),
            "b44a" => Ok(Self::B44a),
            "dwaa" => Ok(Self::Dwaa),
            "dwab" => Ok(Self::Dwab),
            _ => Err(vork_scene::diagnostics::unknown("exr_codec", value,
                ["none", "rle", "zips", "zip", "piz", "pxr24", "b44", "b44a", "dwaa", "dwab"])),
        }
    }
}
impl ExrCodec {
    pub(crate) fn encoding(self) -> Encoding {
        Encoding {
            compression: match self {
                Self::None => Compression::Uncompressed,
                Self::Rle => Compression::RLE,
                Self::Zips => Compression::ZIP1,
                Self::Zip => Compression::ZIP16,
                Self::Piz => Compression::PIZ,
                Self::Pxr24 => Compression::PXR24,
                Self::B44 => Compression::B44,
                Self::B44a => Compression::B44A,
                Self::Dwaa => Compression::DWAA(None),
                Self::Dwab => Compression::DWAB(None),
            },
            blocks: Blocks::ScanLines,
            line_order: LineOrder::Increasing,
        }
    }
}
