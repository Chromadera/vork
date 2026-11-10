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
fn duplicate_environments_refuse_with_named_alternative() {
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
        assert!(!output.status.success(), "{stderr}");
        for expected in ["world_a", "world_b", "additional environment lights are unavailable", "precomposite"] {
            assert!(stderr.contains(expected), "{profile}: {stderr}");
        }
        assert!(!image.exists());
        assert!(!dir.join("still.png.meta.json").exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "renders images; requires a real GPU, pinned Slang and OIDN"]
fn failed_producer_stops_the_range_without_publishing_successors() {
    let dir = std::env::temp_dir().join(format!("vork-refused-frame-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    // Frame 1 (t=0.25) alone has infinite environment radiance.
    std::fs::write(&scene, serde_json::json!({
        "version":2,
        "programs":{"world":{"source":"struct Inputs {}; float3 environment(EnvironmentContext ctx, Inputs inputs){ return ctx.time>0.2 && ctx.time<0.3 ? float3(asfloat(0x7f800000u)) : float3(0.2,0.3,0.4); }"}},
        "materials":{"world":{"program":"world"}},
        "nodes":[{"type":"camera"},{"type":"light","shape":"environment","material":"world"}],
        "render":{"fps":4,"duration":0.75,"resolution":[8,8],"background":"environment"}
    }).to_string()).unwrap();
    let frames = dir.join("frames");
    let output = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("render").arg(&scene).args(["--profile", "std/path", "--set", "samples=1", "--set", "denoise=false", "--out"])
        .arg(&frames).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("environment radiance must be finite"), "{stderr}");
    assert!(!frames.join("000001.png").exists(), "failed producer wrote an image: {stderr}");
    assert!(!frames.join("000002.png").exists(), "queued successor escaped publication failure: {stderr}");
    assert!(!frames.join("meta.json").exists(), "an incomplete output publishes no meta.json");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "renders images; requires a real GPU and pinned Slang"]
fn dropped_light_samples_warn_and_publish_without_diagnostics() {
    let dir = std::env::temp_dir().join(format!("vork-dropped-light-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, serde_json::json!({
        "version":2,
        "programs":{"bad":{"source":"import vork; struct Inputs {}; Closure surface(ShadingContext ctx, Inputs inputs) { return unlit(float3(0.)); } Closure lit(ShadingContext ctx, Inputs inputs, Lighting light) { return unlit(ctx.time < 0.1 ? float3(asfloat(0x7f800000u)) : float3(0.)); }"}},
        "materials":{"bad":{"program":"bad"}},
        "buffers":{"p":{"type":"vec3","data":[[-100,-100,-2],[100,-100,-2],[0,100,-2]]}},
        "geometry":{"triangle":{"attributes":{"POSITION":"p"},"indices":{"type":"u32","data":[0,1,2]}}},
        "nodes":[{"type":"camera"},{"type":"mesh","geometry":"triangle","materials":["bad"]}],
        "render":{"fps":4,"duration":0.5,"resolution":[8,8]}
    }).to_string()).unwrap();
    for diagnostics in [false, true] {
        let frames = dir.join(if diagnostics { "diagnostics" } else { "frames" });
        let mut command = Command::new(env!("CARGO_BIN_EXE_vork"));
        command.arg("render").arg(&scene).args(["--profile", "std/path", "--set", "samples=1", "--set", "denoise=false", "--out"]).arg(&frames);
        if diagnostics { command.arg("--diagnostics"); }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(frames.join("meta.json")).unwrap()).unwrap();
        assert_eq!(metadata["frames"][0]["dropped_light_samples"], 64, "{metadata}");
        assert_eq!(metadata["frames"][1]["dropped_light_samples"], 0, "counter resets between exposures: {metadata}");
        assert!(stderr.contains("warning: frame 0: 64 non-finite light samples dropped"), "{stderr}");
        assert!(!stderr.contains("warning: frame 1:"), "{stderr}");
        for frame in 0..2 { assert!(frames.join(format!("{frame:06}.png")).exists()); }
        if diagnostics {
            let timing: serde_json::Value = serde_json::from_slice(&std::fs::read(frames.join("timings.json")).unwrap()).unwrap();
            assert_eq!(timing["samples"][0]["dropped_light_samples"], 64);
            assert_eq!(timing["samples"][1]["dropped_light_samples"], 0);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(unix)]
#[ignore = "renders images; requires a supported GPU and pinned Slang"]
fn file_program_imports_ignore_unreadable_scene_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vork-import-directories-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("palette.slang"), "public float3 palette(){return float3(0.2,0.4,0.8);}").unwrap();
    std::fs::write(dir.join("surface.slang"), "import palette; struct Inputs{}; Closure surface(ShadingContext ctx, Inputs inputs){return unlit(palette());}").unwrap();
    std::fs::write(dir.join("scene.json"), serde_json::json!({
        "version":2,
        "programs":{"paint":{"file":"surface.slang"}},
        "nodes":[{"type":"camera","translation":[0,0,3],"target":[0,0,0]},
            {"type":"mesh","geometry":{"std":"rounded_box"},"materials":[{"program":"paint"}]}],
        "render":{"resolution":[16,16]}
    }).to_string()).unwrap();
    let mut blocked_directories = Vec::new();
    let mut renders = Vec::new();
    for (index, layout) in ["blocked", "one/two/blocked", "home/Library"].iter().enumerate() {
        let shot = dir.join(index.to_string());
        std::fs::create_dir(&shot).unwrap();
        for file in ["palette.slang", "surface.slang", "scene.json"] {
            std::fs::copy(dir.join(file), shot.join(file)).unwrap();
        }
        let blocked = shot.join(layout);
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0)).unwrap();
        blocked_directories.push(blocked);
        for relative in [false, true] {
            let output_directory = dir.join(format!("output-{index}-{relative}"));
            std::fs::create_dir(&output_directory).unwrap();
            let image = output_directory.join("frame.png");
            let child = Command::new(env!("CARGO_BIN_EXE_vork"))
                .current_dir(&shot)
                .env("VORK_CACHE_DIR", dir.join(format!("cache-{index}-{relative}")))
                .arg("render").arg(if relative { "scene.json".into() } else { shot.join("scene.json") })
                .args(["--profile", "std/draft", "--set", "samples=1", "--set", "denoise=false", "--out"])
                .arg(&image).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped())
                .spawn().unwrap();
            renders.push((layout, relative, image, child));
        }
    }
    // Run the six independent cold renders together. Keep permissions blocked
    // until every process exits, including both path spellings of each scene.
    let outputs: Vec<_> = renders.into_iter().map(|(layout, relative, image, child)| {
        (layout, relative, image, child.wait_with_output().unwrap())
    }).collect();
    for blocked in blocked_directories {
        std::fs::set_permissions(blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (layout, relative, image, output) in outputs {
        assert!(output.status.success(), "{layout}, relative={relative}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(image.exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
