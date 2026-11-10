use std::{fs, process::Command};
fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vork")).current_dir(dir).args(args).output().unwrap()
}
#[test]
fn offline_starter_and_authoring_variants_preserve_user_files() {
    let root = std::env::temp_dir().join(format!("vork-init-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for language in ["json", "typescript", "python"] {
        let output = run(&root, &["init", language, "--language", language]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let project = root.join(language);
        let scene: serde_json::Value = serde_json::from_slice(&fs::read(project.join("scene.json")).unwrap()).unwrap();
        assert_eq!(scene["version"], 2);
        assert_eq!(scene["$schema"], concat!("https://raw.githubusercontent.com/voyager-so/vork/v", env!("CARGO_PKG_VERSION"), "/packages/ts/scene.schema.json"));
        assert_eq!(scene["nodes"].as_array().unwrap().len(), 6);
        let readme = fs::read_to_string(project.join("README.md")).unwrap();
        assert!(readme.contains("vork help"));
        assert!(!project.join("AGENTS.md").exists());
        if language == "typescript" {
            assert!(fs::read_to_string(project.join("scene.ts")).unwrap().contains(scene["$schema"].as_str().unwrap()));
            let package: serde_json::Value = serde_json::from_slice(&fs::read(project.join("package.json")).unwrap()).unwrap();
            assert_eq!(package["type"], "module");
            assert!(readme.contains("LOCALAPPDATA\\Programs\\vork\\sdk"));
            assert!(readme.contains("$HOME/.local/lib/vork/sdk"));
            assert!(readme.contains("Node 22.18+"));
            assert!(readme.contains("node --experimental-strip-types scene.ts"));
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.contains("Node 22.18+"));
            assert!(stdout.contains("node --experimental-strip-types scene.ts"));
        }
        if language == "python" {
            assert!(fs::read_to_string(project.join("scene.py")).unwrap().contains(scene["$schema"].as_str().unwrap()));
            assert!(readme.contains("LOCALAPPDATA\\Programs\\vork\\python"));
            assert!(readme.contains("$HOME/.local/lib/vork/python"));
            assert!(readme.contains("Python 3.11+"));
            assert!(readme.contains("python3 -m venv .venv"));
            assert!(readme.contains(".venv\\Scripts\\python scene.py"));
        }
        fs::write(project.join("scene.json"), "user work").unwrap();
        let again = run(&project, &["init"]);
        assert_eq!(again.status.code(), Some(2));
        assert_eq!(fs::read_to_string(project.join("scene.json")).unwrap(), "user work");
    }
    let conflict = root.join("conflict");
    fs::create_dir(&conflict).unwrap();
    fs::write(conflict.join("README.md"), "notes").unwrap();
    assert!(!run(&conflict, &["init"]).status.success());
    assert!(!conflict.join("scene.json").exists());
    let empty = root.join("current");
    fs::create_dir(&empty).unwrap();
    assert!(run(&empty, &["init"]).status.success());
    assert!(!run(&root, &["init", "invalid", "--language", "rust"]).status.success());
    assert!(!root.join("invalid").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filesystem_failure_is_not_an_argument_refusal() {
    let root = std::env::temp_dir().join(format!("vork-init-file-parent-{}", std::process::id()));
    fs::write(&root, "file, not directory").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vork")).arg("init").arg(root.join("child")).output().unwrap();
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    fs::remove_file(root).unwrap();
}

#[cfg(unix)]
#[test]
fn denied_parent_is_a_filesystem_error() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("vork-init-denied-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    if fs::create_dir(root.join("probe")).is_ok() {
        // Privileged test runners can bypass mode bits.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&root).unwrap();
        return;
    }
    let output = Command::new(env!("CARGO_BIN_EXE_vork")).arg("init").arg(root.join("child")).output().unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(output.status.code(), Some(5), "{output:?}");
}

#[test]
fn help_init_and_usage_errors() {
    let dir = std::env::temp_dir();
    for args in [vec!["help", "init"], vec!["help", "init", "--all"]] {
        let output = run(&dir, &args);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("vork init"));
    }
    for args in [vec!["help", "missing-topic"], vec!["help", "field"], vec!["help", "field", "missing.field"], vec!["help", "init", "extra"]] {
        let output = run(&dir, &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("write stdout"));
    }
}

#[test]
fn typescript_package_conflict_preserves_project() {
    let root = std::env::temp_dir().join(format!("vork-init-package-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("package.json"), "user package").unwrap();
    assert_eq!(run(&root, &["init", "--language", "typescript"]).status.code(), Some(2));
    assert_eq!(fs::read_to_string(root.join("package.json")).unwrap(), "user package");
    assert!(!root.join("scene.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn setup_identifies_broken_vulkan_driver_file() {
    let root = std::env::temp_dir().join(format!("vork-broken-icd-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let manifest = root.join("nvidia_icd.json");
    fs::write(&manifest, r#"{"file_format_version":"1.0.0","ICD":{"library_path":"./missing-driver.so","api_version":"1.3.0"}}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vork"))
        .arg("setup").env("VK_DRIVER_FILES", &manifest).env("VK_ICD_FILENAMES", &manifest)
        .env("XDG_RUNTIME_DIR", &root).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(4), "{stderr}");
    assert!(stderr.contains(&manifest.display().to_string()), "{stderr}");
    assert!(stderr.contains("missing-driver.so, which cannot load as a Vulkan driver"), "{stderr}");
    assert!(!stderr.contains("--gpus all"), "{stderr}");
    fs::remove_dir_all(root).unwrap();
}
