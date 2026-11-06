use std::process::Command;

fn program_enum_help() {
    use std::{fmt::Write, path::Path};
    use vork_slang::{Abi, Compiler};
    let abi = Abi {
        aov_count: 0,
        context: Abi::spec_context(),
        closure: vork_slang::openpbr_fields(),
    };
    let path = Path::new("../../libraries/std/camera.slang");
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-env-changed=VORK_SLANGC");
    let source = std::fs::read_to_string(path).expect("std/camera source");
    let camera = Compiler::discover()
        .compile_camera(&abi, &source, path, "helpcamera")
        .expect("reflect std/camera inputs");
    let mut text = String::new();
    for field in camera.inputs.fields {
        if let Some(enumeration) = field.enumeration {
            let tokens = enumeration
                .cases
                .iter()
                .map(|case| case.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|");
            writeln!(
                text,
                "std/camera.{}: enum {tokens}; {} (JSON tokens).",
                field.name, enumeration.ty
            )
            .unwrap();
        }
    }
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("program-enums.txt"),
        text,
    )
    .unwrap();
}

fn main() {
    program_enum_help();
    // Resolve the worktree HEAD on every build, including packed refs and detached HEADs.
    println!("cargo:rerun-if-changed=build.rs");
    for arg in ["HEAD", "refs", "packed-refs"] {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--git-path", arg])
            .output()
        {
            if output.status.success() {
                println!(
                    "cargo:rerun-if-changed={}",
                    String::from_utf8_lossy(&output.stdout).trim()
                );
            }
        }
    }
    println!("cargo:rerun-if-env-changed=VORK_BUILD_COMMIT");
    let commit = std::env::var("VORK_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .map(|commit| commit.trim().to_owned())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| "source-archive".into());
    println!(
        "cargo:rustc-env=VORK_VERSION={} (commit {}; API v2)",
        std::env::var("CARGO_PKG_VERSION").expect("Cargo package version"),
        commit
    );
}
