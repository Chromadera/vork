//! A single low-frequency observer for terminal and durable redirected progress.
use std::{io::Write, sync::mpsc, thread, time::{Duration, Instant}};
use vork_slang::progress;

pub struct Monitor {
    stop: mpsc::Sender<bool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn line(start: Instant) -> String {
    let (s, done) = progress::snapshot();
    let mut line = format!("[{:.0}s] {}", start.elapsed().as_secs_f64(), s.phase);
    if s.total > 0 {
        line.push_str(&format!(" {done}/{} ({:.0}%)", s.total, 100. * done as f64 / s.total as f64));
    }
    if s.samples_total > 0 {
        line.push_str(&format!("; sample passes recorded {}/{}, GPU completed {}{}", s.samples_recorded, s.samples_total, s.samples_done.load(std::sync::atomic::Ordering::Relaxed).min(s.samples_total),
            if s.samples_recorded == s.samples_total { "; waiting for GPU/output" } else { "" }));
    }
    let compiling = s.shaders_started > s.shaders_done || s.pipelines_started > s.pipelines_done;
    if (s.shaders_started > 0 || s.pipelines_started > 0) && (compiling || s.total == 0) {
        line.push_str(&format!("; shader jobs {}/{}; driver pipelines {}/{} complete (discovered so far)",
            s.shaders_done, s.shaders_started, s.pipelines_done, s.pipelines_started));
    }
    line
}
impl Monitor {
    pub fn start(phase: &'static str) -> Self {
        progress::enable(phase);
        let (stop, receive) = mpsc::channel();
        let start = Instant::now();
        // Print before spawning so even initialization is visibly underway.
        let _ = writeln!(std::io::stderr(), "{}", line(start));
        let worker = thread::spawn(move || {
            let interval = Duration::from_secs(5);
            loop {
                let finished = match receive.recv_timeout(interval) {
                    Ok(success) => Some(success),
                    Err(mpsc::RecvTimeoutError::Disconnected) => Some(false),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                };
                let mut stderr = std::io::stderr().lock();
                let suffix = if let Some(success) = finished {
                    if success { "; done" } else { "; failed" }
                } else { "" };
                let _ = writeln!(stderr, "{}{suffix}", line(start));
                let _ = stderr.flush();
                if finished.is_some() { break; }
            }
        });
        Self { stop, worker: Some(worker) }
    }
    pub fn finish(mut self) { self.join(true); }
    fn join(&mut self, success: bool) {
        if let Some(worker) = self.worker.take() {
            let _ = self.stop.send(success);
            let _ = worker.join();
        }
    }
}
impl Drop for Monitor { fn drop(&mut self) { self.join(false); } }
