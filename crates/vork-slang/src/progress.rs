//! Optional process progress, enabled by the CLI. No output or observer thread in library users.
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}};

#[derive(Clone, Default)]
pub struct State {
    pub phase: &'static str,
    pub total: usize,
    pub samples_done: Arc<AtomicUsize>,
    pub samples_recorded: usize,
    pub samples_total: usize,
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
        state.samples_recorded = 0;
        state.samples_total = 0;
        COMPLETED.store(0, Ordering::Relaxed);
    }
}
/// Recording progress is distinct from completed GPU work; never call it samples rendered.
pub fn sample_pass(recorded: usize, total: usize) {
    if let Some(state) = STATE.get() {
        let mut state = state.lock().unwrap();
        if recorded == 0 { state.samples_done = Arc::default(); }
        state.samples_recorded = recorded;
        state.samples_total = total;
    }
}
/// A frame-local counter: callbacks from older frames cannot update a newer frame.
pub fn sample_counter() -> Option<Arc<AtomicUsize>> {
    STATE.get().map(|state| state.lock().unwrap().samples_done.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_does_not_count_as_a_delivered_frame_and_resets_at_phase_changes() {
        enable("test");
        phase("Rendering frames", 2);
        sample_pass(32, 64);
        let (state, done) = snapshot();
        assert_eq!(done, 0);
        assert_eq!((state.samples_recorded, state.samples_total), (32, 64));
        let old_frame = sample_counter().unwrap();
        sample_pass(0, 32);
        old_frame.store(64, Ordering::Relaxed);
        assert_eq!(sample_counter().unwrap().load(Ordering::Relaxed), 0);
        completed(1);
        phase("Finishing output", 0);
        let (state, done) = snapshot();
        assert_eq!(done, 0);
        assert_eq!((state.samples_recorded, state.samples_total), (0, 0));
    }
}
