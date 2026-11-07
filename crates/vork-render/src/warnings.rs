//! Visible losses are render results, not debug logging. A shot owns its collector;
//! installation workers explicitly inherit it, so concurrent shots stay isolated.
use std::{cell::RefCell, sync::{Arc, Mutex}};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RenderWarning {
    pub code: &'static str,
    /// Authored object, program, binding or value that requested the effect.
    pub requested: serde_json::Value,
    /// Exactly what was omitted or substituted, including the retained behaviour.
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
    let warning = RenderWarning { code, requested, dropped };
    let fresh = CURRENT.with(|current| {
        let collector = current.borrow();
        let Some(collector) = collector.as_ref() else { return true };
        let mut warnings = collector.0.lock().unwrap();
        if warnings.contains(&warning) { false } else { warnings.push(warning.clone()); true }
    });
    if fresh {
        // No ANSI or TTY detection: the same conspicuous record survives logs.
        eprintln!("\nWARNING: REQUESTED EFFECT DROPPED [{}]\n  requested: {}\n  dropped: {}\n", warning.code, warning.requested, warning.dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_unnamed_environments_keep_distinct_authored_locations() {
        let scene = vork_scene::v2::Scene::from_json(br#"{"version":2,"nodes":[{"type":"camera"},{"type":"light","shape":"environment"},{"type":"light","shape":"environment"},{"type":"light","shape":"environment"}]}"#).unwrap();
        let collector = Collector::default();
        let _scope = collector.enter();
        let settings = crate::environment_node::Settings::from_scene(&scene).unwrap();
        assert_eq!(settings.source, Some(1));
        let warnings = collector.snapshot();
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].requested["node_index"], 2);
        assert_eq!(warnings[1].requested["node_index"], 3);
    }

    #[test]
    fn replaced_ray_surface_reports_its_authored_animation_before_removing_it() {
        let mut scene = vork_scene::v2::from_test_value(serde_json::json!({
            "version":2,"nodes":[{"type":"camera"}],
            "render":{"duration":1,"settings":{"integrator":"path"}},
            "materials":{"animated":{"program":{"source":"struct Inputs {float amount=0.;}; Closure surface(ShadingContext c,Inputs i) {Closure s={};return s;}"},
                "inputs":{"geometry_opacity":{"attribute":"alpha"},"amount":0}}},
            "animation":[{"tracks":[{"path":["materials","animated","inputs","amount"],"keys":[[0,0],[1,2]]}]}]
        })).unwrap();
        let collector = Collector::default();
        let _scope = collector.enter();
        crate::rt::normalize_materials(&mut scene);
        assert!(scene.animation[0].tracks.as_ref().unwrap().is_empty());
        let warnings = collector.snapshot();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "ray_surface_attribute");
        assert_eq!(warnings[0].requested["tracks"][0]["path"][1], "animated");
        assert_eq!(warnings[0].requested["surface"]["inputs"]["geometry_opacity"]["attribute"], "alpha");
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

pub(crate) fn additional_environment(node: &vork_scene::v2::Node, first: &str, index: usize) {
    dropped("additional_environment", serde_json::json!({"node": node, "node_index":index, "retained_environment":first}),
        format!("additional environment light at node {index} ({:?}) and its light-property tracks; retaining first environment {first}", node.name));
}
