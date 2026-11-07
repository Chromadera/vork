use std::process::Command;

#[test]
#[ignore = "renders images; requires a supported GPU, pinned Slang and OIDN"]
fn implicit_still_notice_requires_written_output() {
    let dir = std::env::temp_dir().join(format!("vork-range-diagnostics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    let image = dir.join("still.png");
    for (size, explicit) in [(6, false), (0, false), (1, true)] {
        let _ = std::fs::remove_file(&image);
        std::fs::write(&scene, serde_json::json!({
            "version":2,"nodes":[{"type":"camera","name":"cam"}],
            "materials":{"glow":{"program":{"file":concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/compositor-effects/glare.slang")},"inputs":{"size":size}}},
            "render":{"duration":1,"resolution":[8,8],"compositor":["glow"]}
        }).to_string()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_vork"));
        command.arg("render").arg(&scene).args(["--profile","std/draft","--out"]).arg(&image);
        if explicit { command.args(["--at","0"]); }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), size != 6, "{stderr}");
        assert_eq!(image.exists(), size != 6, "{stderr}");
        assert_eq!(stderr.contains("rendered t=0"), size != 6 && !explicit, "{stderr}");
        if size == 6 {
            for expected in ["render.compositor[0]", "glow", "received 6", "[0, 1]"] {
                assert!(stderr.contains(expected), "missing {expected}: {stderr}");
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "renders images; requires a real GPU, pinned Slang and OIDN"]
fn duplicate_environments_warn_render_and_return_authored_requests() {
    let dir = std::env::temp_dir().join(format!("vork-environment-diagnostics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, serde_json::json!({"version":2,"nodes":[
        {"type":"camera"},
        {"type":"light","shape":"environment","name":"world_a"},
        {"type":"node","children":[{"type":"light","shape":"environment","name":"world_b","energy":2}]}
    ],"render":{"resolution":[8,8]}}).to_string()).unwrap();
    for profile in ["std/draft", "std/final", "std/path"] {
        let image = dir.join("still.png");
        let output = Command::new(env!("CARGO_BIN_EXE_vork"))
            .arg("render").arg(&scene).args(["--profile", profile, "--set", "samples=1", "--set", "denoise=false", "--out"])
            .arg(&image).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        for expected in ["WARNING: REQUESTED EFFECT DROPPED", "world_a", "world_b", "additional_environment"] {
            assert!(stderr.contains(expected), "{profile}: {stderr}");
        }
        assert!(image.exists());
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("still.png.meta.json")).unwrap()).unwrap();
        let warnings = metadata["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0]["code"], "additional_environment");
        assert_eq!(warnings[0]["requested"]["node"]["name"], "world_b");
        assert_eq!(warnings[0]["requested"]["node"]["energy"].as_f64(), Some(2.));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
