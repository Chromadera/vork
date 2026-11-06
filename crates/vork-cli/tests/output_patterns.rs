use std::process::Command;

#[test]
fn png_frame_patterns_are_usage_errors_even_for_one_still() {
    let dir = std::env::temp_dir().join(format!("vork-output-patterns-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"nodes":[{"type":"camera"}]}"#).unwrap();
    for name in ["frame_####.png", "frame_%04d.png"] {
        let path = dir.join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_vork"))
            .args(["render"]).arg(&scene).arg("--out").arg(&path)
            .args(["--at", "0"]).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("NNNNNN.png") && stderr.contains("%06d.exr"), "{stderr}");
        assert!(!path.exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn png_still_paths_preserve_literal_characters() {
    let dir = std::env::temp_dir().join(format!("vork-literal-output-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8]},"nodes":[{"type":"camera"}]}"#).unwrap();
    for name in ["renders/review#2/still.png", "renders/100%/still.png", "part#1.png", "100%.png"] {
        for at in [false, true] {
            let path = dir.join(name);
            let mut command = Command::new(env!("CARGO_BIN_EXE_vork"));
            command.arg("render").arg(&scene).arg("--out").arg(&path).args(["--profile", "std/draft"]);
            if at { command.args(["--at", "0"]); }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{}: {}", path.display(), String::from_utf8_lossy(&output.stderr));
            assert!(path.is_file());
            let metadata = path.with_file_name(format!("{}.meta.json", path.file_name().unwrap().to_string_lossy()));
            let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(metadata).unwrap()).unwrap();
            assert_eq!(metadata["profile"]["level"], 0, "{metadata}");
            std::fs::remove_file(path).unwrap();
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
