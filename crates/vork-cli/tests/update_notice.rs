//! Subprocess checks use only prewritten cache files; no registry is contacted.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str, cache: &[u8]) -> Self {
        let root =
            std::env::temp_dir().join(format!("vork-update-cli-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("vork")).unwrap();
        fs::write(root.join("vork/update-notice.json"), cache).unwrap();
        Self(root)
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vork"));
        cmd.env_remove("CI")
            .env_remove("VORK_NO_UPDATE_NOTIFIER")
            .env("XDG_CACHE_HOME", &self.0)
            .env("LOCALAPPDATA", &self.0);
        // macOS uses HOME/Library/Caches rather than XDG_CACHE_HOME.
        #[cfg(target_os = "macos")]
        {
            cmd.env("HOME", &self.0);
            fs::create_dir_all(self.0.join("Library/Caches/vork")).unwrap();
            fs::copy(
                self.0.join("vork/update-notice.json"),
                self.0.join("Library/Caches/vork/update-notice.json"),
            )
            .unwrap();
        }
        cmd
    }
    fn run(&self, args: &[&str], switch: Option<&str>) -> Output {
        let mut cmd = self.command();
        cmd.args(args);
        if let Some(switch) = switch {
            cmd.env(switch, "");
        }
        cmd.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cache() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "checked": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        "release": {"version": "999.0.0", "vorkUpdate": {"reason": "fixes a regression", "regression": true}}
    })).unwrap()
}
#[test]
fn redirected_stderr_notice_preserves_stdout_and_success_or_failure_status() {
    let fixture = Fixture::new("redirected", &cache());
    for args in [
        vec!["help"],
        vec!["check", "__missing_update_notice_scene__.json"],
    ] {
        let baseline = fixture.run(&args, Some("VORK_NO_UPDATE_NOTIFIER"));
        let enabled = fixture.run(&args, None);
        assert_eq!(enabled.status.code(), baseline.status.code());
        assert_eq!(enabled.stdout, baseline.stdout);
        let stderr = String::from_utf8(enabled.stderr).unwrap();
        assert!(
            stderr.contains("Upgrade recommended (regression fix): 999.0.0"),
            "{stderr}"
        );
        assert!(stderr.contains(concat!("installed ", env!("CARGO_PKG_VERSION"))));
        assert!(stderr.contains("run `"));
        assert!(stderr.contains("`; fixes a regression"));
    }
}
#[test]
fn ci_and_off_switch_disable_notices_and_workers_even_with_empty_values() {
    for switch in ["CI", "VORK_NO_UPDATE_NOTIFIER"] {
        let fixture = Fixture::new(switch, &cache());
        let output = fixture.run(&["help"], Some(switch));
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        // A corrupt cache would be refreshed if the switch did not suppress work.
        fs::write(fixture.0.join("vork/update-notice.json"), b"broken").unwrap();
        let output = fixture.run(&["help"], Some(switch));
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            fs::read(fixture.0.join("vork/update-notice.json")).unwrap(),
            b"broken"
        );
    }
}
#[test]
fn failed_registry_response_is_silent_on_later_run() {
    let fixture = Fixture::new("offline", &serde_json::to_vec(&serde_json::json!({
        "checked": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), "release": null
    })).unwrap());
    let output = fixture.run(&["help"], None);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn verbose_worker_distinguishes_contention_and_failure_without_changing_exit_status() {
    let fixture = Fixture::new("worker-status", &cache());
    #[cfg(target_os = "macos")]
    let path = fixture.0.join("Library/Caches/vork/update-notice.json");
    #[cfg(not(target_os = "macos"))]
    let path = fixture.0.join("vork/update-notice.json");
    // command() also prepares the macOS cache before we open it.
    let mut command = fixture.command();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    struct HeldLock(fs::File);
    impl Drop for HeldLock {
        fn drop(&mut self) {
            let _ = self.0.unlock();
        }
    }
    file.lock().unwrap();
    let lock = HeldLock(file);
    let output = command
        .args(["--internal-update-notice-worker", "--verbose"])
        .output()
        .unwrap();
    drop(lock);
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("skipped: another process is refreshing")
    );
    // Replace the cache parent with a regular file for a deterministic I/O error.
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    fs::write(path.parent().unwrap(), b"not a directory").unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("update notifier: failed:"));
}
