//! Test-only consumer for a whole-scene capture without a monitor in its own scene.
use serde_json::{Value, json};

pub fn compositor(scene: &mut Value, image: &str) -> Value {
    scene["programs"]["test_capture_copy"] = json!({"source":
        "struct Inputs {Image source;}; float4 composite(CompositeContext c,Inputs i) {return compositor_load(i.source,int2(c.uv*c.size));}"});
    let material = format!("test_capture_copy_{image}");
    scene["materials"][&material] = json!({"program":"test_capture_copy","inputs":{"source":{"image":image}}});
    json!([material])
}
