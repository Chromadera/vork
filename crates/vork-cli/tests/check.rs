use serde_json::{Value, json};
use std::{io::Write, process::{Command, Output, Stdio}};

fn check(scene: &Value) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vork"))
        .args(["check", "-", "--profile", "std/path"])
        // A check must not consult device selection or try this invalid ICD.
        .env("VORK_GPU", "not-a-device")
        .env("VK_ICD_FILENAMES", "/missing/check-must-not-open-vulkan.json")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(scene.to_string().as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}
fn starter() -> Value {
    serde_json::from_str(vork_scene::STARTER_SCENE).unwrap()
}
fn failure(scene: &Value, code: i32, text: &str) {
    let output = check(scene);
    let error = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(code), "{error}");
    assert!(error.contains(text), "expected {text:?}: {error}");
}

#[test]
fn check_expands_and_installs_without_a_gpu() {
    let mut scene = starter();
    for name in ["box", "sphere", "grid", "cylinder", "cone", "torus", "capsule", "revolve"] {
        let geometry = if name == "revolve" {
            json!({"std":name,"profile":[[0,0],[1,0],[1,1],[0,1]]})
        } else { json!({"std":name}) };
        scene["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh","geometry":geometry}));
    }
    let output = check(&scene);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("compatibility_04"));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Scene checked successfully");
}

#[test]
fn check_rejects_schema_program_assets_and_timeline() {
    let mut scene = starter();
    scene["render"]["typo"] = json!(true);
    failure(&scene, 3, "typo");
    let mut scene = starter();
    scene["render"]["display"] = json!({"program":"std/missing"});
    failure(&scene, 3, "std/missing");
    let mut scene = starter();
    scene["render"]["duration"] = json!(-1);
    failure(&scene, 3, "render requires positive dimensions/time");
    let mut scene = starter();
    scene["nodes"].as_array_mut().unwrap().push(json!({"type":"asset","file":"missing-check-asset.glb"}));
    failure(&scene, 5, "missing-check-asset.glb");
}

#[test]
fn check_uses_fog_feature_admission() {
    let mut scene = starter();
    scene["materials"] = json!({"fog":{"program":{"source":"import vork; struct Inputs {}; MediumCoefficients medium(MediumContext ctx, Inputs inputs) { MediumCoefficients m = {}; m.scattering = float3(0.02); return m; }"}}});
    scene["nodes"].as_array_mut().unwrap().push(json!({"type":"light", "shape":"environment", "medium":"fog"}));
    // Implicit geometry is admitted without opening a GPU or compiling native shaders.
    scene["buffers"]["grid"] = json!({"type":"f32", "dims":[2,2,2], "data":[0,0,0,0,0,0,0,0]});
    scene["geometry"] = json!({"field":{"topology":"implicit", "attributes":{"density":"grid"}}});
    let output = check(&scene);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    scene["materials"]["custom"] = json!({"program":{"source":"import vork; struct Inputs {}; Closure surface(ShadingContext c, Inputs i) { return Closure(); }"}});
    scene["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh", "geometry":"field", "materials":["custom"]}));
    failure(&scene, 3, "fog with custom surface callbacks is not yet supported");
}

#[test]
fn file_base_and_optional_shader_compilation_share_render_installation() {
    let dir = std::env::temp_dir().join(format!("vork-check-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("display.slang"), "import vork; struct Inputs {}; float3 display(DisplayContext ctx, Inputs inputs) { return ctx.color; }").unwrap();
    let mut scene = starter();
    scene["render"]["display"] = json!({"program":{"file":"display.slang"}});
    std::fs::write(dir.join("scene.json"), scene.to_string()).unwrap();
    for shaders in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vork"));
        command.arg("check").arg(dir.join("scene.json"))
            .env("VORK_CACHE_DIR", dir.join("cache"))
            .env("VORK_GPU", "not-a-device");
        if shaders { command.arg("--shaders"); }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        for file in std::fs::read_dir(dir.join("cache")).unwrap() {
            let name = file.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.contains("pipeline-keys") && !name.contains("shader-warm"), "check wrote driver cache ledger {name}");
            if !shaders { assert!(!name.contains("slang-variant"), "default check generated native shader {name}"); }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn water_example_checks_without_a_gpu() {
    let output = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("check")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/water/scene.json"))
        .env("VORK_GPU", "not-a-device")
        .env("VK_ICD_FILENAMES", "/missing/check-must-not-open-vulkan.json")
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Scene checked successfully");
}

#[test]
fn missing_fonts_are_reported_together() {
    let scene = json!({"version":2,"nodes":[{"type":"camera"}],"geometry":{
        "one":{"std":"text","body":"first","font":"missing-first.ttf"},
        "two":{"std":"text","body":"second","font":"missing-second.otf"}}});
    let output = check(&scene);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    for name in ["missing-first.ttf", "missing-second.otf", "restore its font"] {assert!(error.contains(name), "{error}");}
}
