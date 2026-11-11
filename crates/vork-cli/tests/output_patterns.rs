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
#[ignore = "renders images; requires a supported GPU, pinned Slang and OIDN"]
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

#[test]
#[ignore = "renders images; requires a supported GPU, pinned Slang and OIDN"]
fn failed_overwrite_cannot_retain_success_metadata() {
    let dir = std::env::temp_dir().join(format!("vork-failed-overwrite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8]},"nodes":[{"type":"camera"}]}"#).unwrap();
    let out = dir.join("frames");
    let render = |diagnostics| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vork"));
        command.arg("render").arg(&scene).arg("--out").arg(&out)
            .args(["--profile", "std/draft", "--at", "0"]);
        for name in ["still.png", "float-%06d.exr", "video.mp4"] {
            command.arg("--out").arg(dir.join(name));
        }
        if diagnostics { command.arg("--diagnostics"); }
        command.output().unwrap()
    };
    let first = render(false);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let manifests = [out.join("meta.json"), dir.join("still.png.meta.json"),
        dir.join("float.exr.meta.json"), dir.join("video.mp4.meta.json")];
    for manifest in &manifests { assert!(manifest.is_file(), "{}", manifest.display()); }
    // An output I/O failure, after a previous successful run.
    std::fs::create_dir(out.join("timings.json")).unwrap();
    let failed = render(true);
    assert!(!failed.status.success());
    for manifest in &manifests {
        assert!(!manifest.exists(), "failed overwrite retained {}", manifest.display());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "renders images; requires a supported GPU, pinned Slang and OIDN"]
fn shorter_overwrite_and_benchmarks_do_not_claim_old_frames() {
    let dir = std::env::temp_dir().join(format!("vork-short-overwrite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8],"duration":1,"fps":4},"nodes":[{"type":"camera"}]}"#).unwrap();
    let out = dir.join("frames");
    let render = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_vork"))
            .arg("render").arg(&scene).arg("--out").arg(&out)
            .args(["--profile", "std/draft"]).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    };
    render(&["--channels", "color,depth,camera", "--diagnostics"]);
    assert!(out.join("000003.png").exists());
    assert!(out.join("camera.json").exists());
    std::fs::write(out.join("notes.txt"), "keep").unwrap();
    let preserved: Vec<_> = [0, 2, 3].into_iter().flat_map(|frame| {
        [".png", ".depth.png"].into_iter().map(move |suffix| format!("{frame:06}{suffix}"))
    }).map(|name| {
        let bytes = std::fs::read(out.join(&name)).unwrap();
        (name, bytes)
    }).collect();
    render(&["--frame-start", "1", "--frame-end", "1"]);
    assert!(out.join("000001.png").exists());
    for (name, bytes) in &preserved {
        assert_eq!(std::fs::read(out.join(name)).unwrap(), *bytes, "preserve {name}");
    }
    for name in ["000001.depth.png", "camera.json", "timings.json"] {
        assert!(!out.join(name).exists(), "stale {name}");
    }
    let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["frames"].as_array().unwrap().len(), 1);
    assert_eq!(meta["frames"][0]["index"], 1);
    // A shorter whole clip still replaces the entire generated namespace.
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8],"duration":0.25,"fps":4},"nodes":[{"type":"camera"}]}"#).unwrap();
    render(&[]);
    assert!(out.join("000000.png").exists());
    for name in ["000001.png", "000002.png", "000003.png", "000000.depth.png", "000001.depth.png", "000002.depth.png", "000003.depth.png", "camera.json", "timings.json"] {
        assert!(!out.join(name).exists(), "stale {name}");
    }
    assert!(out.join("notes.txt").exists());
    let meta = std::fs::read(out.join("meta.json")).unwrap();
    for flag in ["--no-write", "--gpu-only"] {
        render(&["--diagnostics", flag]);
        assert_eq!(std::fs::read(out.join("meta.json")).unwrap(), meta);
        assert!(!out.join("timings.json").exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "renders images; requires a supported GPU, pinned Slang and OIDN"]
fn interrupted_overwrite_has_no_completion_record() {
    use std::os::unix::process::ExitStatusExt;
    let dir = std::env::temp_dir().join(format!("vork-interrupted-overwrite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8],"duration":10000,"fps":24},"nodes":[{"type":"camera"}]}"#).unwrap();
    let out = dir.join("frames");
    let command = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_vork"));
        c.arg("render").arg(&scene).arg("--out").arg(&out).args(["--profile", "std/draft"]);
        c
    };
    let first = command().args(["--frame-end", "99"]).output().unwrap();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    assert!(out.join("meta.json").exists());
    let previous = std::fs::read(out.join("000001.png")).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&std::fs::read(&scene).unwrap()).unwrap();
    changed["render"]["background"] = serde_json::json!([0.8, 0.1, 0.1]);
    std::fs::write(&scene, serde_json::to_vec(&changed).unwrap()).unwrap();
    let replaced = || std::fs::read(out.join("000001.png"))
        .is_ok_and(|bytes| !bytes.is_empty() && bytes != previous);
    let log = std::fs::File::create(dir.join("interrupted.log")).unwrap();
    let mut child = command().stdout(std::process::Stdio::null()).stderr(log).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !replaced() && std::time::Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let wrote = replaced();
    let signalled = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(signalled.success(), "{}", std::fs::read_to_string(dir.join("interrupted.log")).unwrap());
    let status = child.wait().unwrap();
    assert!(wrote, "{}", std::fs::read_to_string(dir.join("interrupted.log")).unwrap());
    assert_eq!(status.signal(), Some(2), "expected SIGINT: {}", std::fs::read_to_string(dir.join("interrupted.log")).unwrap());
    assert!(!out.join("meta.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "renders images; requires a Windows console, supported GPU, pinned Slang and OIDN"]
fn ctrl_break_closes_sequence_and_reports_saved_output() {
    use std::os::windows::process::CommandExt;
    #[link(name = "Kernel32")]
    unsafe extern "system" { fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32; }
    let dir = std::env::temp_dir().join(format!("vork-break-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"render":{"resolution":[8,8],"duration":10000,"fps":24},"nodes":[{"type":"camera"}]}"#).unwrap();
    let out = dir.join("frames");
    let log_path = dir.join("interrupt.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("render").arg(scene).arg("--out").arg(&out).args(["--profile", "std/previs"])
        .creation_flags(0x00000200) // CREATE_NEW_PROCESS_GROUP: target only the renderer.
        .stdout(log.try_clone().unwrap()).stderr(log).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !out.join("000001.png").exists() && std::time::Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(out.join("000001.png").exists(), "renderer did not begin writing");
    assert_ne!(unsafe { GenerateConsoleCtrlEvent(1, child.id()) }, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if std::time::Instant::now() > deadline { child.kill().unwrap(); panic!("Ctrl+Break did not finish output"); }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(130));
    let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("meta.json")).unwrap()).unwrap();
    let frames = metadata["frames"].as_array().unwrap();
    assert!(!frames.is_empty() && frames.len() < 240000);
    for frame in frames {
        let path = out.join(format!("{:06}.png", frame["index"].as_u64().unwrap()));
        assert!(std::fs::read(path).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.contains("Interrupted: saved"));
    assert!(log.contains("output files closed cleanly"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn png16_requires_a_png_colour_destination() {
    for args in [vec!["--out", "movie.mp4"], vec!["--out", "frame%06d.exr"],
        vec!["--out", "frame.png", "--channels", "depth"],
        vec!["--out", "frame.png", "--no-write", "--diagnostics"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_vork"))
            .args(["render", "missing.json", "--png-depth", "16"]).args(args).output().unwrap();
        assert_eq!(output.status.code(),Some(2),"{}",String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires PNG colour output"));
    }
}
