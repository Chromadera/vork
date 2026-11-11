use std::process::Command;

#[test]
#[ignore = "renders images; requires a real GPU with same-device OIDN, pinned Slang and OIDN"]
fn obsolete_denoiser_environment_does_not_change_render_defaults() {
    let dir = std::env::temp_dir().join(format!("vork-denoiser-environment-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("scene.json");
    std::fs::write(&scene, r#"{"version":2,"nodes":[{"type":"camera"}],"render":{"resolution":[8,8]}}"#).unwrap();
    // VORK_BIN allows this contract to exercise an ordinary release binary,
    // independently of Cargo's test-support feature unification.
    let binary = std::env::var_os("VORK_BIN").unwrap_or_else(|| env!("CARGO_BIN_EXE_vork").into());
    for profile in ["std/draft", "std/final"] {
        let render = |off: bool| {
            let image = dir.join(if off {"off.png"} else {"default.png"});
            let mut command = Command::new(&binary);
            command.arg("render").arg(&scene).args(["--profile", profile, "--out"]).arg(&image)
                .env_remove("VORK_GPU_DENOISER");
            if off { command.env("VORK_GPU_DENOISER", "off"); }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(image.with_extension("png.meta.json")).unwrap()).unwrap()
        };
        let baseline = render(false);
        assert_eq!(baseline["denoiser"], "gpu", "requires a working same-device OIDN: {baseline}");
        let off = render(true);
        assert_eq!(off["denoiser"], baseline["denoiser"]);
        assert_eq!(off["settings"], baseline["settings"]);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
