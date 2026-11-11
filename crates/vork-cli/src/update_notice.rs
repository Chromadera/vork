//! Best-effort update advice. Network runs only in a background worker after
//! command completion; a command can only display a previously cached result.
//! Worker status is silent unless the caller requests --verbose.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DAY: u64 = 24 * 60 * 60;
const LIMIT: u64 = 64 * 1024;
const WORKER: &str = "--internal-update-notice-worker";
const REGISTRY: &str = "https://registry.npmjs.org/@voyager-so/vork/latest";

#[derive(Default, Deserialize, Serialize)]
struct Cache {
    checked: u64,
    release: Option<Release>,
}
#[derive(Deserialize, Serialize)]
struct Release {
    version: String,
    #[serde(default, rename = "vorkUpdate")]
    advice: Advice,
}
#[derive(Default, Deserialize, Serialize)]
struct Advice {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    regression: bool,
}
fn enabled(ci: bool, off: bool) -> bool {
    !ci && !off
}
fn allowed() -> bool {
    enabled(
        std::env::var_os("CI").is_some(),
        std::env::var_os("VORK_NO_UPDATE_NOTIFIER").is_some(),
    )
}
fn path() -> Option<PathBuf> {
    let absolute_env = |key| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    #[cfg(windows)]
    let root = absolute_env("LOCALAPPDATA")?;
    #[cfg(target_os = "macos")]
    let root = absolute_env("HOME")?.join("Library/Caches");
    #[cfg(not(any(windows, target_os = "macos")))]
    let root =
        absolute_env("XDG_CACHE_HOME").or_else(|| Some(absolute_env("HOME")?.join(".cache")))?;
    Some(root.join("vork/update-notice.json"))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read(file: &mut File) -> Option<Cache> {
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
fn fresh(cache: &Cache, time: u64) -> bool {
    cache.checked > 0 && time.saturating_sub(cache.checked) < DAY
}
fn notice(cache: &Cache, installed: &str, command: &str) -> Option<String> {
    let release = cache.release.as_ref()?;
    if release.version.len() > 128 {
        return None;
    }
    let latest = semver::Version::parse(&release.version).ok()?;
    let current = semver::Version::parse(installed).ok()?;
    // Development builds and prereleases must not be advertised as stable releases.
    if !latest.pre.is_empty() || !current.pre.is_empty() || !latest.cmp_precedence(&current).is_gt()
    {
        return None;
    }
    let reason: String = release
        .advice
        .reason
        .chars()
        .map(|c| {
            if c.is_control() || c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .take(240)
        .collect();
    let heading = if release.advice.regression {
        "Upgrade recommended (regression fix)"
    } else {
        "Update available"
    };
    Some(format!(
        "vork: {heading}: {latest} (installed {current}); run `{command}`{}",
        if reason.trim().is_empty() {
            String::new()
        } else {
            format!("; {}", reason.trim())
        }
    ))
}

fn upgrade_command(exe: Option<&Path>) -> &'static str {
    if let Some(exe) = exe {
        for ancestor in exe.ancestors() {
            if ancestor.file_name().is_some_and(|n| n == "node_modules") {
                return if ancestor
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "lib" || (cfg!(windows) && (n == "npm" || n == "nodejs")))
                {
                    "npm install -g @voyager-so/vork@latest"
                } else {
                    "npm install @voyager-so/vork@latest"
                };
            }
            if ancestor
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("vork_native_"))
            {
                return "python -m pip install --upgrade vork";
            }
        }
    }
    #[cfg(windows)]
    {
        "powershell -NoProfile -Command \"& ([scriptblock]::Create((irm https://github.com/voyager-so/vork/releases/latest/download/install.ps1)))\""
    }
    #[cfg(not(windows))]
    {
        "curl -fsSL https://github.com/voyager-so/vork/releases/latest/download/install.sh | sh"
    }
}

// Detached Windows workers cannot use inherited console stderr. Verbose
// workers share the parent's console; quiet workers remain detached and hidden.
#[cfg(any(windows, test))]
fn worker_creation_flags(verbose: bool) -> u32 {
    if verbose { 0 } else { 0x08000000 | 0x00000008 } // CREATE_NO_WINDOW | DETACHED_PROCESS
}

pub(crate) fn after_command() {
    if !allowed() {
        return;
    }
    let Some(path) = path() else {
        return;
    };
    let cache = File::open(&path)
        .ok()
        .and_then(|mut f| read(&mut f))
        .unwrap_or_default();
    if let Some(line) = notice(
        &cache,
        env!("CARGO_PKG_VERSION"),
        upgrade_command(std::env::current_exe().ok().as_deref()),
    ) {
        // Broken stderr is advisory too: never let it change the command status.
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    }
    if fresh(&cache, now()) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let verbose = std::env::args_os().any(|arg| arg == "--verbose");
    let mut child = Command::new(exe);
    child
        .arg(WORKER)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
    if verbose {
        child.arg("--verbose");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(worker_creation_flags(verbose));
    }
    // Dropping Child does not wait. The worker owns no render handles or outputs.
    if let Err(error) = child.spawn() {
        if verbose {
            let _ = writeln!(
                std::io::stderr().lock(),
                "vork: update notifier: failed: {error}"
            );
        }
    }
}

pub(crate) fn worker_entry() -> bool {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER)) {
        return false;
    }
    if allowed() {
        if let Some(path) = path() {
            let result = refresh(&path, now(), fetch);
            if std::env::args_os().any(|arg| arg == "--verbose") {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "vork: update notifier: {}",
                    refresh_message(&result)
                );
            }
        }
    }
    true
}
fn fetch() -> std::io::Result<Option<Release>> {
    // No auth, identifiers, scene data or installed-version headers. No redirects
    // or retries; a fixed package metadata request with a fixed user-agent only.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .https_only(true)
        .max_redirects(0)
        .user_agent("vork-update-notifier")
        .build()
        .into();
    let mut response = agent.get(REGISTRY).call().map_err(std::io::Error::other)?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "update registry response exceeds size limit",
        ));
    }
    parse_release(&bytes).map(Some).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid update registry response",
        )
    })
}
fn parse_release(bytes: &[u8]) -> Option<Release> {
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    let mut release: Release = serde_json::from_slice(bytes).ok()?;
    if release.version.len() > 128 {
        return None;
    }
    let version = semver::Version::parse(&release.version).ok()?;
    if !version.pre.is_empty() {
        return None;
    }
    // Persist only bounded data, so a large registry reason cannot make our own
    // cache unreadable and accidentally defeat the daily attempt timestamp.
    release.advice.reason = release.advice.reason.chars().take(240).collect();
    Some(release)
}
fn save(file: &mut File, cache: &Cache) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(cache)?;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&bytes)?;
    file.flush()
}
// Explicit unlock releases ownership even while a forked child retains the FD.
struct RefreshLock(File);
impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
#[derive(Debug, PartialEq, Eq)]
enum Refresh {
    Refreshed,
    Fresh,
    Busy,
}
fn refresh_message(result: &std::io::Result<Refresh>) -> String {
    match result {
        Ok(Refresh::Refreshed) => "refreshed".into(),
        Ok(Refresh::Fresh) => "skipped: cache is fresh".into(),
        Ok(Refresh::Busy) => "skipped: another process is refreshing".into(),
        Err(error) => format!("failed: {error}"),
    }
}
fn acquired(result: Result<(), TryLockError>) -> std::io::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error),
    }
}
fn refresh(
    path: &Path,
    time: u64,
    fetch: impl FnOnce() -> std::io::Result<Option<Release>>,
) -> std::io::Result<Refresh> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    if !acquired(file.try_lock())? {
        return Ok(Refresh::Busy);
    }
    let mut lock = RefreshLock(file);
    let file = &mut lock.0;
    let cache = read(file).unwrap_or_default();
    if fresh(&cache, time) {
        return Ok(Refresh::Fresh);
    }
    // Claim BEFORE fetching: failures and interrupted workers also wait a day.
    save(
        file,
        &Cache {
            checked: time,
            release: None,
        },
    )?;
    let release = fetch()?;
    save(
        file,
        &Cache {
            checked: time,
            release,
        },
    )?;
    Ok(Refresh::Refreshed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn refresh(
        path: &Path,
        time: u64,
        fetch: impl FnOnce() -> Option<Release>,
    ) -> std::io::Result<Refresh> {
        super::refresh(path, time, || Ok(fetch()))
    }
    #[test]
    fn verbose_windows_worker_preserves_inherited_console_access() {
        // Console diagnostics require neither DETACHED_PROCESS nor CREATE_NO_WINDOW.
        let no_console = 0x00000008 | 0x08000000;
        assert_eq!(worker_creation_flags(true) & no_console, 0);
        assert_eq!(worker_creation_flags(false) & no_console, no_console);
    }
    #[test]
    fn fetch_failure_is_reported_and_throttled() {
        let path =
            std::env::temp_dir().join(format!("vork-update-fetch-error-{}", std::process::id()));
        let result = super::refresh(&path, DAY, || Err(std::io::Error::other("offline")));
        assert_eq!(refresh_message(&result), "failed: offline");
        assert_eq!(
            refresh(&path, DAY + 1, || panic!("failed attempts throttled")).unwrap(),
            Refresh::Fresh
        );
        fs::remove_file(path).unwrap();
    }
    // Only async-signal-safe libc calls run in the forked child. It keeps all
    // inherited descriptors alive until the parent drops this pipe, even on panic.
    #[cfg(target_os = "linux")]
    struct Forked {
        pid: libc::pid_t,
        pipe: [libc::c_int; 2],
    }
    #[cfg(target_os = "linux")]
    impl Forked {
        fn holding_descriptors() -> Self {
            let mut pipe = [0; 2];
            assert_eq!(
                unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) },
                0
            );
            let pid = unsafe { libc::fork() };
            assert!(pid >= 0);
            if pid == 0 {
                unsafe {
                    libc::close(pipe[1]);
                    let mut byte = 0u8;
                    libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
                    libc::_exit(0);
                }
            }
            Self { pid, pipe }
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for Forked {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.pipe[1]);
                libc::waitpid(self.pid, std::ptr::null_mut(), 0);
                libc::close(self.pipe[0]);
            }
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn fork_inside_refresh_does_not_extend_ownership() {
        let path = std::env::temp_dir().join(format!("vork-update-fork-{}", std::process::id()));
        let mut child = None;
        assert_eq!(
            refresh(&path, DAY, || {
                child = Some(Forked::holding_descriptors());
                parse_release(br#"{"version":"999.0.0"}"#)
            })
            .unwrap(),
            Refresh::Refreshed
        );
        let mut fetched = 0;
        assert_eq!(
            refresh(&path, DAY * 2, || {
                fetched += 1;
                None
            })
            .unwrap(),
            Refresh::Refreshed
        );
        assert_eq!(
            refresh(&path, DAY * 2 + 1, || panic!("fresh cache")).unwrap(),
            Refresh::Fresh
        );
        assert_eq!(fetched, 1);
        let cache = read(&mut File::open(&path).unwrap()).unwrap();
        assert_eq!(cache.checked, DAY * 2);
        assert!(notice(&cache, "0.5.0", "upgrade").is_none());
        drop(child);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn unlocks_with_retained_descriptors_on_return_error_and_unwind() {
        let path = std::env::temp_dir().join(format!("vork-update-unlock-{}", std::process::id()));
        for exit in 0..3 {
            let mut retained = None;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> std::io::Result<()> {
                    let file = File::create(&path)?;
                    file.lock()?;
                    let lock = RefreshLock(file);
                    retained = Some(lock.0.try_clone()?);
                    match exit {
                        0 => Ok(()),
                        1 => Err(std::io::Error::other("injected I/O failure")),
                        _ => panic!("injected unwind"),
                    }
                },
            ));
            let file = File::open(&path).unwrap();
            file.try_lock()
                .expect("scope must release even with a retained descriptor");
            let _lock = RefreshLock(file);
            drop(retained);
        }
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn fake_registry_cached_for_later_runs_and_failures_throttled() {
        let path = std::env::temp_dir().join(format!(
            "vork-update-test-{}/cache.json",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(path.parent().unwrap());
        let fake = || {
            parse_release(br#"{"version":"0.5.2","vorkUpdate":{"reason":"fixes black frames\nWindows\u001b","regression":true}}"#)
        };
        refresh(&path, DAY, fake).unwrap();
        refresh(&path, DAY + 1, || {
            panic!("network must not run within a day")
        })
        .unwrap();
        let cache = read(&mut File::open(&path).unwrap()).unwrap();
        let line = notice(&cache, "0.5.0", "npm install -g @voyager-so/vork@latest").unwrap();
        assert!(line.contains("regression fix"));
        assert!(line.contains("0.5.2 (installed 0.5.0)"));
        assert!(line.contains("npm install -g @voyager-so/vork@latest"));
        assert_eq!(line.lines().count(), 1);
        assert!(!line.contains('\u{1b}'));
        assert!(notice(&cache, "0.5.2", "npm install -g @voyager-so/vork@latest").is_none());
        assert!(notice(&cache, "0.6.0", "npm install -g @voyager-so/vork@latest").is_none());
        assert!(
            notice(
                &cache,
                "0.5.1-dev.1",
                "npm install -g @voyager-so/vork@latest"
            )
            .is_none()
        );
        refresh(&path, DAY * 2, || None).unwrap();
        refresh(&path, DAY * 2 + 1, || panic!("offline attempts throttled")).unwrap();
        assert!(
            notice(
                &read(&mut File::open(&path).unwrap()).unwrap(),
                "0.5.0",
                "upgrade"
            )
            .is_none()
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn switches_and_versions() {
        assert!(enabled(false, false));
        assert!(!enabled(true, false));
        assert!(!enabled(false, true));
        for version in ["garbage", "0.5.2-beta.1", "0.4.0"] {
            let cache = Cache {
                checked: DAY,
                release: Some(Release {
                    version: version.into(),
                    advice: Advice::default(),
                }),
            };
            assert!(notice(&cache, "0.5.0", "npm install -g @voyager-so/vork@latest").is_none());
        }
        assert!(fresh(
            &Cache {
                checked: DAY * 2,
                release: None
            },
            DAY
        ));
    }
    #[test]
    fn registry_responses_are_validated_and_bounded() {
        for bytes in [
            b"bad".as_slice(),
            br#"{"version":"not-semver"}"#,
            br#"{"version":"0.6.0-beta.1"}"#,
        ] {
            assert!(parse_release(bytes).is_none());
        }
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":"0.5.2", "vorkUpdate":{"reason":"x".repeat(500)}}),
        )
        .unwrap();
        assert_eq!(parse_release(&bytes).unwrap().advice.reason.len(), 240);
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":format!("0.5.2+{}", "x".repeat(500))}),
        )
        .unwrap();
        assert!(parse_release(&bytes).is_none());
    }
    #[test]
    fn ordinary_notice_and_install_sources() {
        let release: Release = serde_json::from_str(r#"{"version":"0.5.2"}"#).unwrap();
        let line = notice(
            &Cache {
                checked: DAY,
                release: Some(release),
            },
            "0.5.0",
            "upgrade",
        )
        .unwrap();
        assert_eq!(
            line,
            "vork: Update available: 0.5.2 (installed 0.5.0); run `upgrade`"
        );
        assert_eq!(
            upgrade_command(Some(Path::new(
                "/project/node_modules/@voyager-so/vork-linux-x64/runtime/vork"
            ))),
            "npm install @voyager-so/vork@latest"
        );
        assert_eq!(
            upgrade_command(Some(Path::new(
                "/prefix/lib/node_modules/@voyager-so/vork-linux-x64/runtime/vork"
            ))),
            "npm install -g @voyager-so/vork@latest"
        );
        assert_eq!(
            upgrade_command(Some(Path::new(
                "/venv/site-packages/vork_native_linux_x64/runtime/vork"
            ))),
            "python -m pip install --upgrade vork"
        );
        for directory in ["npm", "nodejs"] {
            let exe = PathBuf::from(format!(
                "/prefix/{directory}/node_modules/@voyager-so/vork-windows-x64/runtime/vork.exe"
            ));
            assert_eq!(
                upgrade_command(Some(&exe)),
                if cfg!(windows) {
                    "npm install -g @voyager-so/vork@latest"
                } else {
                    "npm install @voyager-so/vork@latest"
                }
            );
        }
        let release: Release = serde_json::from_str(r#"{"version":"0.5.0+build"}"#).unwrap();
        assert!(
            notice(
                &Cache {
                    checked: DAY,
                    release: Some(release)
                },
                "0.5.0",
                "upgrade"
            )
            .is_none()
        );
    }
    #[test]
    fn malformed_and_oversized_cache_recovers_without_network_in_test() {
        let path = std::env::temp_dir().join(format!("vork-update-bad-{}", std::process::id()));
        for bytes in [b"bad json".to_vec(), vec![b' '; LIMIT as usize + 1]] {
            fs::write(&path, bytes).unwrap();
            assert!(read(&mut File::open(&path).unwrap()).is_none());
            refresh(&path, DAY, || {
                serde_json::from_str(r#"{"version":"0.5.2"}"#).ok()
            })
            .unwrap();
            assert!(
                notice(
                    &read(&mut File::open(&path).unwrap()).unwrap(),
                    "0.5.0",
                    "upgrade"
                )
                .is_some()
            );
        }
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn contention_is_skipped_and_errors_are_failed() {
        let path = std::env::temp_dir().join(format!("vork-update-locked-{}", std::process::id()));
        let file = File::create(&path).unwrap();
        file.lock().unwrap();
        let lock = RefreshLock(file);
        let result = refresh(&path, DAY, || panic!("locked cache must not fetch"));
        assert_eq!(result.as_ref().unwrap(), &Refresh::Busy);
        assert_eq!(
            refresh_message(&result),
            "skipped: another process is refreshing"
        );
        let result = acquired(Err(TryLockError::Error(std::io::Error::other(
            "lock failure",
        ))))
        .map(|_| Refresh::Refreshed);
        assert_eq!(refresh_message(&result), "failed: lock failure");
        assert!(refresh(&path.join("impossible"), DAY, || panic!()).is_err());
        drop(lock);
        fs::remove_file(path).unwrap();
    }
}
