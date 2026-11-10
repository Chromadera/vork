//! Bundled Slang libraries, shared by validation and every compiler workspace.
//!
//! A module named `<library>/<module>` is imported as `import <library>.<module>;`;
//! a material program is referenced as `"program": "<library>/<module>"`.
pub struct Module {
    pub name: &'static str,
    pub source: &'static str,
    /// Whether the module exports an authored material stage rather than library functions.
    pub material: bool,
}
#[derive(Clone, Copy)]
pub enum ProgramImage {
    Embedded(&'static [u8]),
    /// Path relative to the canonical executable's directory.
    File(&'static str),
}

pub struct Library {
    pub name: &'static str,
    /// `vork help <library>`.
    pub help: &'static str,
    pub modules: &'static [Module],
    /// Render profiles `<library>/<name>`: settings-tier data files.
    pub profiles: &'static [(&'static str, &'static str)],
    /// Data a bundled program reads through an `Image` input the author leaves
    /// unbound: (program, input, OpenEXR source). It ships with the library,
    /// like the program's source.
    pub images: &'static [(&'static str, &'static str, ProgramImage)],
}

macro_rules! modules {
    ($library:literal: $($module:literal $material:literal),* $(,)?) => {
        &[$(Module {
            name: concat!($library, "/", $module),
            source: include_str!(concat!("../../../libraries/", $library, "/", $module, ".slang")),
            material: $material,
        }),*]
    };
}

/// Blender AgX samples owned by the std display program.
const AGX_LUT: &[u8] = include_bytes!("../../../libraries/std/agx.exr");

pub const LIBRARIES: &[Library] = &[
    // Fundamental and hard to write correctly. `vork setup` prebuilds its programs.
    Library {
        name: "std",
        help: include_str!("../../../libraries/std/HELP.txt"),
        modules: modules!("std":
            "openpbr_core" false, "texture" false, "normal_filter" false, "hair" true, "hair_bsdf" false,
            "camera" true, "sky" true, "environment_image" true, "grid" false, "pnanovdb" false,
            "color" false,
            "agx" true, "standard" true, "display" false),
        profiles: &[
            ("std/previs", include_str!("../../../libraries/std/profiles/previs.json")),
            ("std/draft", include_str!("../../../libraries/std/profiles/draft.json")),
            ("std/final", include_str!("../../../libraries/std/profiles/final.json")),
            ("std/path", include_str!("../../../libraries/std/profiles/path.json")),
        ],
        images: &[("std/agx", "lut", ProgramImage::Embedded(AGX_LUT))],
    },
    // Blender node and socket compatibility for the bridge and Blender-faithful scenes.
    Library {
        name: "blender",
        help: include_str!("../../../libraries/blender/HELP.txt"),
        modules: modules!("blender":
            "nodes" false, "volume" false, "white_balance" false,
            "texture_coordinate" false, "color" false, "vector" false, "procedurals" false, "filmic" true, "display_lut" false),
        profiles: &[],
        images: &[("blender/filmic", "lut", ProgramImage::File("libraries/blender/filmic.exr"))],
    },
    Library {
        name: "fluid",
        help: include_str!("../../../libraries/fluid/HELP.txt"),
        modules: modules!("fluid": "config" false, "solver" false, "surface" false, "whitewater" false, "film" false, "particles" false, "volume" false, "reconstruction" false),
        profiles: &[],
        images: &[],
    },
    Library {
        name: "cloth",
        help: include_str!("../../../libraries/cloth/HELP.txt"),
        modules: modules!("cloth": "solver" false, "body" false, "self" false),
        profiles: &[],
        images: &[],
    },


];

impl Library {
    pub fn programs(&self) -> impl Iterator<Item = &'static str> {
        self.modules.iter().filter(|module| module.material).map(|module| module.name)
    }
}
/// The library `vork setup` prebuilds, so a scene using only its programs compiles
/// nothing at render time. Other libraries compile on first use.
pub const PREBUILT: &str = "std";
pub fn prebuilt_programs() -> impl Iterator<Item = &'static str> {
    library(PREBUILT).expect("prebuilt library").programs()
}

/// A bundled profile's file text by `<library>/<name>`.
pub fn profile(name: &str) -> Option<&'static str> {
    LIBRARIES.iter().flat_map(|library| library.profiles).find(|(key, _)| *key == name).map(|(_, text)| *text)
}
pub fn profile_names() -> impl Iterator<Item = &'static str> {
    LIBRARIES.iter().flat_map(|library| library.profiles).map(|(name, _)| *name)
}

pub fn library(name: &str) -> Option<&'static Library> {
    LIBRARIES.iter().find(|library| library.name == name)
}
/// Every bundled module of every library.
pub fn modules() -> impl Iterator<Item = &'static Module> {
    LIBRARIES.iter().flat_map(|library| library.modules).chain(compatibility_modules()).chain(test_modules())
}
pub fn module_source(name: &str) -> Option<&'static str> {
    modules().find(|module| module.name == name).map(|module| module.source)
}
/// The library data a bundled program's unbound `Image` input reads.
pub fn program_image(program: &str, input: &str) -> Option<ProgramImage> {
    LIBRARIES.iter().flat_map(|library| library.images)
        .find(|(name, field, _)| *name == program && *field == input).map(|(_, _, image)| *image)
}
/// The built-in table material and every bundled material program.
pub fn material_programs<'a>() -> impl Iterator<Item = &'a str> {
    std::iter::once("std/openpbr").chain(modules().filter(|module| module.material).map(|module| module.name))
}

// Disabled release conveniences remain available only to internal media tests.
fn test_modules() -> impl Iterator<Item = &'static Module> {
    #[cfg(feature = "test-support")]
    let modules = modules!("std": "fog" true);
    #[cfg(not(feature = "test-support"))]
    let modules: &[Module] = &[];
    #[cfg(feature = "test-support")]
    let blender = modules!("blender": "principled_volume" true);
    #[cfg(not(feature = "test-support"))]
    let blender: &[Module] = &[];
    modules.iter().chain(blender)
}

// Undocumented compatibility through 0.5.x. Never include these in library help or setup.
pub const COMPATIBILITY_MODULES: &[&str] = &["std/bloom", "post/white_balance", "std/procedurals", "std/vector", "std/particles"];
fn compatibility_modules() -> impl Iterator<Item = &'static Module> {
    modules!("std": "bloom" true, "procedurals" false, "vector" false, "particles" false).iter()
        .chain(modules!("post": "white_balance" true))
}

#[cfg(test)]
#[test]
fn compatibility_expires_in_06() {
    let version: Vec<u32> = env!("CARGO_PKG_VERSION").split('.').take(2).map(|n| n.parse().unwrap()).collect();
    if version.as_slice() >= [0, 6].as_slice() {
        let remaining: Vec<_> = COMPATIBILITY_MODULES.iter().filter(|name| module_source(name).is_some()).collect();
        assert!(remaining.is_empty(), "remove expired 0.4 compatibility modules: {remaining:?}");
    }
}
