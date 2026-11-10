//! Exercise the CLI's successful process-exit boundary, which skips GPU drops.
#![cfg(target_os = "macos")]

#[test]
#[ignore = "requires a Metal GPU with binary archives and pinned Slang"]
fn successful_render_reclaims_metal_archive_backing_files() {
    use std::{fs, process::{Command, Stdio}};
    let root = std::env::temp_dir().join(format!("vork-archive-exit-{}", std::process::id()));
    let temporary = root.join("temporary");
    fs::create_dir_all(&temporary).unwrap();
    let scene = root.join("scene.json");
    fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8]},"nodes":[{"type":"camera"}]}"#).unwrap();
    // A successful setup persists an archive. The render must then stage its
    // input even if background compilation prevents a new serialization.
    let setup = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("setup")
        .env("WGPU_BACKEND", "metal")
        .env("VORK_CACHE_DIR", root.join("cache"))
        .env("TMPDIR", &temporary)
        .output().unwrap();
    assert!(setup.status.success(), "{}", String::from_utf8_lossy(&setup.stderr));
    assert!(String::from_utf8_lossy(&setup.stdout).contains("Pipeline cache: empty"),
        "a fresh Metal binary archive is required: {}", String::from_utf8_lossy(&setup.stdout));
    let child = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("render").arg(&scene).arg("--out").arg(root.join("frame.png"))
        .args(["--profile", "std/draft", "--set", "samples=1", "--set", "denoise=false"])
        .env("WGPU_BACKEND", "metal")
        .env("VORK_CACHE_DIR", root.join("cache"))
        .env("TMPDIR", &temporary)
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let prefix = format!("wgpu-metal-archive-{}-", child.id());
    let output = child.wait_with_output().unwrap();
    let leftovers: Vec<_> = fs::read_dir(&temporary).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with(&prefix))
        .collect();
    let delivered = root.join("frame.png").is_file();
    fs::remove_dir_all(&root).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(delivered, "successful process produced no frame");
    assert!(leftovers.is_empty(), "successful CLI exit left archive backing files: {leftovers:?}");
}
