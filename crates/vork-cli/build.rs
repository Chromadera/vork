use std::process::Command;

fn program_input_help() {
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
        // Read initializers from the bundled source rather than duplicating camera defaults.
        let initializer = source.lines().find_map(|line| {
            let (declaration, value) = line.split_once('=')?;
            (declaration.split_whitespace().last()? == field.name)
                .then(|| value.split_once(';').expect("camera input initializer").0.trim())
        }).expect("bundled camera input has an initializer");
        let default = if field.enumeration.is_some() {
            initializer.rsplit('.').next().unwrap().to_ascii_lowercase()
        } else if let vork_slang::Type::Vector(n) = field.ty {
            let value = initializer.split_once('(').expect("camera vector initializer").1.trim_end_matches(')');
            let value: f64 = value.parse().expect("camera vector splat");
            format!("[{}]", vec![value.to_string(); n as usize].join(","))
        } else if matches!(field.ty, vork_slang::Type::Float) {
            initializer.parse::<f64>().expect("camera numeric initializer").to_string()
        } else { initializer.to_owned() };
        let units = match field.name.as_str() {
            "lens" | "sensor_width" | "sensor_height" => "; mm",
            "ortho_scale" | "clip_start" | "clip_end" | "focus_distance" => "; metres",
            "aperture_rotation" => "; radians",
            _ => "",
        };
        if let Some(enumeration) = field.enumeration {
            let tokens = enumeration
                .cases
                .iter()
                .map(|case| case.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|");
            writeln!(
                text,
                "std/camera.{}: enum {tokens}; {} (JSON tokens); default {default}.",
                field.name, enumeration.ty
            )
            .unwrap();
        } else {
            let ty = match field.ty {
                vork_slang::Type::Float => "number".to_owned(),
                vork_slang::Type::Uint => "uint".to_owned(),
                vork_slang::Type::Bool => "bool".to_owned(),
                vork_slang::Type::Vector(n) => format!("vec{n}"),
                ty => format!("{ty:?}"),
            };
            writeln!(text, "std/camera.{}: {ty}{units}; default {default}.", field.name).unwrap();
        }
    }
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("program-inputs.txt"),
        text,
    )
    .unwrap();
}

fn main() {
    program_input_help();
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
