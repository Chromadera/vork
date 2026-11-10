//! Visible losses and compatibility notices are render results, not debug logging. A shot owns its collector;
//! installation workers explicitly inherit it, so concurrent shots stay isolated.
use std::{cell::RefCell, sync::{Arc, Mutex}};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RenderWarning {
    pub code: &'static str,
    /// Authored object, program, binding or value; runtime losses name the output frame and counters.
    pub requested: serde_json::Value,
    /// Omitted/substituted behaviour, or compatibility removal and rewrite guidance.
    pub dropped: String,
}
#[derive(Clone, Default)]
pub(crate) struct Collector(Arc<Mutex<Vec<RenderWarning>>>);
thread_local! { static CURRENT: RefCell<Option<Collector>> = const { RefCell::new(None) }; }
pub(crate) struct Guard(Option<Collector>);
impl Drop for Guard {
    fn drop(&mut self) { CURRENT.with(|c| *c.borrow_mut() = self.0.take()); }
}
impl Collector {
    pub(crate) fn enter(&self) -> Guard {
        Guard(CURRENT.with(|c| c.replace(Some(self.clone()))))
    }
    pub(crate) fn current() -> Option<Self> { CURRENT.with(|c| c.borrow().clone()) }
    pub(crate) fn snapshot(&self) -> Vec<RenderWarning> { self.0.lock().unwrap().clone() }
}
pub(crate) fn dropped(code: &'static str, requested: serde_json::Value, dropped: String) {
    record(code, requested, dropped);
}
fn record(code: &'static str, requested: serde_json::Value, dropped: String) {
    let warning = RenderWarning { code, requested, dropped };
    let fresh = CURRENT.with(|current| {
        let collector = current.borrow();
        let Some(collector) = collector.as_ref() else { return true };
        let mut warnings = collector.0.lock().unwrap();
        if warnings.contains(&warning) { false } else { warnings.push(warning.clone()); true }
    });
    if fresh {
        // No ANSI or TTY detection: the same conspicuous record survives logs.
        let title = if code == "compatibility_04" { "0.4 COMPATIBILITY" } else { "REQUESTED EFFECT DROPPED" };
        eprintln!("\nWARNING: {title} [{}]\n  requested: {}\n  dropped: {}\n", warning.code, warning.requested, warning.dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provisional_environment_selection_does_not_decide_admission() {
        let scene = vork_scene::v2::Scene::from_json(br#"{"version":2,"nodes":[{"type":"camera"},{"type":"light","shape":"environment"},{"type":"light","shape":"environment"},{"type":"light","shape":"environment"}]}"#).unwrap();
        let collector = Collector::default();
        let _scope = collector.enter();
        let settings = crate::environment_node::Settings::from_scene(&scene).unwrap();
        assert_eq!(settings.source, Some(1));
        let warnings = collector.snapshot();
        assert!(warnings.is_empty());
    }

    #[test]
    fn collectors_isolate_shots_and_inherit_workers_without_duplicate_records() {
        let first = Collector::default();
        let _scope = first.enter();
        dropped("test", serde_json::json!({"node":"a","stage":"lit"}), "lit stage; retaining surface".into());
        let worker = Collector::current().unwrap();
        std::thread::spawn(move || {
            let _scope = worker.enter();
            dropped("test", serde_json::json!({"node":"a","stage":"lit"}), "lit stage; retaining surface".into());
        }).join().unwrap();
        let second = Collector::default();
        { let _scope = second.enter(); dropped("other", serde_json::json!({"node":"b"}), "displacement".into()); }
        assert_eq!(first.snapshot().len(), 1);
        assert_eq!(second.snapshot().len(), 1);
        assert_eq!(first.snapshot()[0].requested["stage"], "lit");
        assert_eq!(serde_json::to_value(first.snapshot()).unwrap()[0]["code"], "test");
    }
}

pub(crate) fn compatibility(name: &str, requested: serde_json::Value) {
    let rewrite = if name == "std/particles" { "an import of fluid.particles" }
        else { "authored Slang" };
    record("compatibility_04", requested, format!("{name} is removed in 0.6; rewrite as {rewrite}; see upgrade notes"));
}
