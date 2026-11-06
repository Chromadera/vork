fn main() {
    println!("cargo:rerun-if-changed=native/constants.h");
    println!("cargo:rerun-if-changed=native/nvenc.c");
    println!("cargo:rerun-if-changed=native/nvcodec");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        cc::Build::new()
            .file("native/nvenc.c")
            .compile("vork_nvenc");
        println!("cargo:rustc-link-lib=dl");
    }
    println!("cargo:rerun-if-changed=native/encode.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/encode.m")
            .flag("-fobjc-arc")
            .compile("vork_encode");
        for framework in [
            "Foundation",
            "AVFoundation",
            "VideoToolbox",
            "CoreMedia",
            "CoreVideo",
            "Metal",
            "IOSurface",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
