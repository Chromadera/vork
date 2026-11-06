fn main() {
    // The private corpus and its test modules are intentionally absent from public exports.
    println!("cargo:rustc-check-cfg=cfg(vork_private_tests)");
    println!("cargo:rerun-if-changed=build.rs");
    if std::path::Path::new("../../scenes").is_dir() {
        println!("cargo:rerun-if-changed=../../scenes");
        println!("cargo:rustc-cfg=vork_private_tests");
    }
}
