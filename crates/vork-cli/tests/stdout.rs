use std::process::{Command, Stdio};

#[test]
fn closed_consumers_terminate_successfully_without_panics() {
    let root = std::env::temp_dir().join(format!("vork-stdout-init-{}", std::process::id()));
    let project = root.to_str().unwrap();
    for args in [vec!["std", "rounded_box", "{}"], vec!["help", "programs", "--all"],
        vec!["init", project], vec!["setup"], vec!["--help"], vec!["--version"]] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vork")).args(&args)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn real_stdout_errors_are_filesystem_failures() {
    let root = std::env::temp_dir().join(format!("vork-full-init-{}", std::process::id()));
    for args in [vec!["std", "rounded_box", "{}"], vec!["help"],
        vec!["init", root.to_str().unwrap()], vec!["setup"], vec!["--help"], vec!["--version"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_vork")).args(&args)
            .stdout(std::fs::File::options().write(true).open("/dev/full").unwrap()).output().unwrap();
        assert_eq!(output.status.code(), Some(5), "{args:?}: {output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"), "{output:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn consumer_can_close_after_reading_a_prefix() {
    use std::io::Read;
    let mut child = Command::new(env!("CARGO_BIN_EXE_vork")).args(["std", "rounded_box", "{}"])
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    stdout.read_exact(&mut [0u8; 1]).unwrap();
    drop(stdout);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn gpu_status_output_uses_the_same_stdout_policy() {
    if std::env::var("VORK_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP GPU stdout test: set VORK_GPU_TESTS=1 on a supported GPU");
        return;
    }
    use std::io::{BufRead, BufReader};
    let mut setup = Command::new(env!("CARGO_BIN_EXE_vork")).arg("setup")
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut headers = BufReader::new(setup.stdout.take().unwrap());
    // CLI version, OS and FFmpeg precede the renderer's setup report.
    for _ in 0..3 { assert!(headers.read_line(&mut String::new()).unwrap() > 0); }
    drop(headers);
    let output = setup.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"), "{output:?}");

    let root = std::env::temp_dir().join(format!("vork-gpu-stdout-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let scene = root.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"nodes":[{"type":"camera"}],"render":{"resolution":[1,1]}}"#).unwrap();
    for full in [false, true] {
        if full && !cfg!(target_os = "linux") { continue; }
        let stdout = if full { Stdio::from(std::fs::File::options().write(true).open("/dev/full").unwrap()) }
            else { Stdio::piped() };
        let mut child = Command::new(env!("CARGO_BIN_EXE_vork")).arg("render").arg(&scene)
            .args(["--profile", "std/draft", "--set", "denoise=false", "--set", "samples=1", "--out"])
            .arg(root.join("frame.png")).stdout(stdout).stderr(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(if full {5} else {0}), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"), "{output:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
