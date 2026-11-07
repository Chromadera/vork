#[path = "../vork-slang/src/registry.rs"]
#[allow(dead_code)]
mod registry;
#[path = "src/object_abi.rs"]
#[allow(dead_code)]
mod object_abi;
#[path = "src/binding_abi.rs"]
#[allow(dead_code)]
mod binding_abi;
use std::{fs, path::Path};
#[path = "build_slang.rs"]
mod slang;
fn hash_tree(path: &Path, hash: &mut blake3::Hasher) {
    let mut entries: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            hash_tree(&path, hash);
        } else {
            let name = path.to_string_lossy();
            let bytes = fs::read(&path).unwrap();
            hash.update(&(name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(&(bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
        }
    }
}
fn main() {
    // Explicit opt-in: directory layout must not change test admission.
    println!("cargo:rustc-check-cfg=cfg(vork_private_tests)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VORK_PRIVATE_TESTS");
    if std::env::var("VORK_PRIVATE_TESTS").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=vork_private_tests");
    }
    // Texture bytes depend on codecs, not unrelated rendering changes or GPU IDs.
    let mut textures = blake3::Hasher::new();
    for file in [
        "src/compressed_textures.rs",
        "src/gpu_bc7.rs",
        "src/constants.rs",
        "shaders/mip.slang",
        "shaders/bc7.slang",
        "shaders/bc7_refine.slangh",
        "../../Cargo.lock",
        "../../dist/slang.json",
    ] {
        println!("cargo:rerun-if-changed={file}");
        textures.update(&fs::read(file).unwrap());
    }
    hash_tree(Path::new("../../vendor/basisu_c_sys"), &mut textures);
    println!("cargo:rerun-if-changed=../../vendor/basisu_c_sys");
    hash_tree(Path::new("shaders/third_party/directxtex"), &mut textures);
    println!("cargo:rerun-if-changed=shaders/third_party/directxtex");
    println!(
        "cargo:rustc-env=VORK_TEXTURE_BUILD_ID={}",
        textures.finalize()
    );
    let mut hash = blake3::Hasher::new();
    // One conservative build identity covers every host generator, pipeline
    // descriptor and shader. Consumer code and Git history are not cache inputs.
    for tree in [
        "src",
        "shaders",
        "../vork-scene/src",
        "../vork-slang/src",
        "../vork-mesh/src",
        "../vork-std/src",
    ] {
        println!("cargo:rerun-if-changed={tree}");
        hash_tree(Path::new(tree), &mut hash);
    }
    let closure = std::env::var("DEP_VORK_OPENPBR_CLOSURE").unwrap();
    println!("cargo:rustc-env=VORK_CLOSURE_WGSL={closure}");
    let out = std::env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out).join("gpu_object.rs"), object_abi::rust()).unwrap();
    let closure_dir = Path::new(&closure).parent().unwrap();
    let (slang, shader_sources) = slang::compile(Path::new(&out), closure_dir);
    // Native Slang output is portable across devices and unrelated host builds.
    // Include shader generators as well as their inputs; pipeline descriptors and
    // scene loading do not participate in this artifact's identity.
    let mut shaders = blake3::Hasher::new();
    shaders.update(slang.as_bytes());
    shaders.update(shader_sources.as_bytes());
    for tree in ["shaders", "../vork-slang/src"] {
        hash_tree(Path::new(tree), &mut shaders);
    }
    for file in [
        closure.as_str(),
        "build_slang.rs",
        "src/slang_forward.rs",
        "src/programs.rs",
        "src/forward_variant.rs",
        "src/surface_native.rs",
        "src/visibility_native.rs",
        "src/camera_native.rs",
        "../../libraries/std/openpbr_core.slang",
    ] {
        let bytes = fs::read(file).unwrap();
        shaders.update(&(bytes.len() as u64).to_le_bytes());
        shaders.update(&bytes);
    }
    println!("cargo:rustc-env=VORK_SHADER_ID={}", shaders.finalize());
    hash.update(slang.as_bytes());
    for file in [
        closure.as_str(),
        "build.rs",
        "build_slang.rs",
        "../../libraries/std/openpbr_core.slang",
        "src/forward_variant.rs",
        "Cargo.toml",
        "../../Cargo.toml",
        "../../Cargo.lock",
        "../vork-mesh/Cargo.toml",
        "../vork-scene/Cargo.toml",
        "../vork-scene/build.rs",
        "../vork-slang/Cargo.toml",
    ] {
        println!("cargo:rerun-if-changed={file}");
        let bytes = fs::read(file).unwrap();
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    // Reflected programs and generated native code depend on these sources,
    // not the host executable's target, optimization level or GPU adapter.
    println!("cargo:rustc-env=VORK_PORTABLE_ID={}", hash.finalize());
    for variable in ["TARGET", "OPT_LEVEL", "CARGO_ENCODED_RUSTFLAGS"] {
        println!("cargo:rerun-if-env-changed={variable}");
        let value = std::env::var(variable).unwrap_or_default();
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    println!("cargo:rustc-env=VORK_BUILD_ID={}", hash.finalize());
}
