//! Scene errors must reach the CLI without leaving device startup detached.
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn invalid_scene_exits_with_validation_error() {
    let dir = std::env::temp_dir().join(format!("vork-invalid-scene-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    // Match the toon ergonomics failure: diagnostics finish while the device
    // startup worker is still inside the graphics loader. Inline buffer data
    // exercises the same parsing/initialization overlap as the original scene.
    let nodes = (0..49)
        .map(|_| serde_json::json!({"type": "mesh", "mesh": "missing"}))
        .collect::<Vec<_>>();
    std::fs::write(
        &scene,
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": 2, "nodes": nodes,
            "buffers": {"positions": {"type": "vec3", "data":
                vec![[0.4408290376865476, 0.4589949493661166, 0.17320508075688773]; 8192]}}
        }))
        .unwrap(),
    )
    .unwrap();
    // Drain diagnostics to a file while polling: a pipe can fill before exit,
    // making the harness itself look like a renderer shutdown deadlock.
    let log = dir.join("stderr.log");
    for _ in 0..32 {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vork"))
            .arg("render")
            .arg(&scene)
            .arg("--out")
            .arg(dir.join("frame.png"))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if start.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!(
                    "scene error hung during shutdown: {}",
                    std::fs::read_to_string(&log).unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "status: {}; stderr: {}",
            output.status,
            std::fs::read_to_string(&log).unwrap()
        );
        let stderr = std::fs::read_to_string(&log).unwrap();
        assert!(stderr.contains("unknown field \"mesh\""), "{stderr}");
        assert!(!dir.join("frame.png").exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
