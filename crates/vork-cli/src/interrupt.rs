//! Windows console callbacks only request cancellation; the render thread closes files.
#[cfg(windows)]
mod windows {
    use std::sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}};
    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    unsafe extern "system" fn control(event: u32) -> i32 {
        // CTRL_C_EVENT and CTRL_BREAK_EVENT. Closing/logoff/shutdown retain OS handling.
        if event > 1 { return 0; }
        if let Some(stop) = STOP.get() {
            if !stop.swap(true, Ordering::Relaxed) {
                eprintln!("Interrupt received: finishing submitted frames and closing output files...");
            }
        }
        1
    }
    pub fn install() -> anyhow::Result<Option<Arc<AtomicBool>>> {
        let stop = STOP.get_or_init(|| Arc::new(AtomicBool::new(false))).clone();
        anyhow::ensure!(unsafe { SetConsoleCtrlHandler(Some(control), 1) } != 0,
            "install console interrupt handler: {}", std::io::Error::last_os_error());
        Ok(Some(stop))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{os::windows::process::CommandExt, time::{Duration, Instant}};
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn GetConsoleCP() -> u32;
            fn AllocConsole() -> i32;
            fn FreeConsole() -> i32;
            fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
        }
        #[test]
        fn console_child() {
            let Some(ready) = std::env::var_os("VORK_TEST_CONSOLE_READY") else { return; };
            let stop = install().unwrap().unwrap();
            std::fs::write(ready, b"ready").unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(stop.load(Ordering::Relaxed), "console callback was not invoked");
        }
        #[test]
        fn native_ctrl_break_reaches_the_console_handler() {
            struct Console(bool);
            impl Drop for Console { fn drop(&mut self) { if self.0 { unsafe { FreeConsole(); } } } }
            let allocated = unsafe { GetConsoleCP() } == 0;
            if allocated { assert_ne!(unsafe { AllocConsole() }, 0, "allocate test console"); }
            let _console = Console(allocated);
            let ready = std::env::temp_dir().join(format!("vork-console-ready-{}", std::process::id()));
            let _ = std::fs::remove_file(&ready);
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "interrupt::windows::tests::console_child", "--nocapture"])
                .env("VORK_TEST_CONSOLE_READY", &ready)
                .creation_flags(0x00000200) // Target only the child process group.
                .spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !ready.exists() && Instant::now() < deadline {
                if child.try_wait().unwrap().is_some() { break; }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !ready.exists() { let _ = child.kill(); let _ = child.wait(); panic!("console child did not initialize"); }
            assert_ne!(unsafe { GenerateConsoleCtrlEvent(1, child.id()) }, 0, "send Ctrl+Break");
            let deadline = Instant::now() + Duration::from_secs(15);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() { break status; }
                if Instant::now() >= deadline { let _ = child.kill(); let _ = child.wait(); panic!("console child did not handle Ctrl+Break"); }
                std::thread::sleep(Duration::from_millis(10));
            };
            let _ = std::fs::remove_file(ready);
            assert!(status.success(), "console child: {status}");
        }

        #[test]
        fn ctrl_c_and_break_request_the_same_clean_shutdown() {
            let stop = STOP.get_or_init(|| Arc::new(AtomicBool::new(false)));
            for event in [0, 1] {
                stop.store(false, Ordering::Relaxed);
                assert_eq!(unsafe { control(event) }, 1);
                assert!(stop.load(Ordering::Relaxed));
            }
            assert_eq!(unsafe { control(2) }, 0);
        }
    }
}
#[cfg(windows)]
pub use windows::install;
#[cfg(not(windows))]
pub fn install() -> anyhow::Result<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> { Ok(None) }
