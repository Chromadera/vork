//! Optional process progress, enabled by the CLI. No output or observer thread in library users.
use std::sync::{Mutex, OnceLock, atomic::{AtomicUsize, Ordering}};

#[derive(Clone, Default)]
pub struct State {
    pub phase: &'static str,
    pub total: usize,
    pub shaders_started: usize,
    pub shaders_done: usize,
    pub pipelines_started: usize,
    pub pipelines_done: usize,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static COMPLETED: AtomicUsize = AtomicUsize::new(0);

pub fn enable(phase: &'static str) {
    *STATE.get_or_init(Default::default).lock().unwrap() = State { phase, ..Default::default() };
    COMPLETED.store(0, Ordering::Relaxed);
}
pub fn phase(phase: &'static str, total: usize) {
    if let Some(state) = STATE.get() {
        let mut state = state.lock().unwrap();
        state.phase = phase;
        state.total = total;
        COMPLETED.store(0, Ordering::Relaxed);
    }
}
/// One atomic update per delivered frame/setup configuration; never called in a sample or GPU loop.
pub fn completed(count: usize) { COMPLETED.fetch_max(count, Ordering::Relaxed); }
pub fn snapshot() -> (State, usize) {
    (STATE.get().expect("progress enabled").lock().unwrap().clone(), COMPLETED.load(Ordering::Relaxed))
}
/// Count Slang invocations and driver pipeline creations, including driver-only work.
pub struct Job { driver: bool, enabled: bool }
impl Job {
    pub fn start(driver: bool) -> Self {
        let enabled = STATE.get().is_some();
        if let Some(state) = STATE.get() {
            let mut state = state.lock().unwrap();
            if driver { state.pipelines_started += 1; } else { state.shaders_started += 1; }
        }
        Self { driver, enabled }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.enabled {
            let mut state = STATE.get().unwrap().lock().unwrap();
            if self.driver { state.pipelines_done += 1; } else { state.shaders_done += 1; }
        }
    }
}
