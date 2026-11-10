// Custom Hair transport tests retain both closure responses independently of
// std/hair's physical scattering.
pub fn with_test_hair(mut scene: serde_json::Value) -> serde_json::Value {
    scene["programs"]["test_hair"] = serde_json::json!({"source": include_str!("../fixtures/hair-model.slang")});
    scene
}

/// Expand flat-box test fixtures without invoking the authored std preprocessor.
#[allow(dead_code)]
pub fn with_box_geometry(mut scene:serde_json::Value)->serde_json::Value {
    let fixture:serde_json::Value=serde_json::from_str(include_str!("../fixtures/analytic.json")).unwrap();
    let mut geometry=fixture["geometry"]["shape_box"].clone();
    for name in geometry["attributes"].as_object_mut().unwrap().values_mut() {*name=name.as_str().unwrap().replace("shape_box_","test_unit_box_").into();}
    geometry["indices"]=serde_json::json!("test_unit_box_indices");
    scene["geometry"]["test_unit_box"]=geometry;
    for (name,buffer) in fixture["buffers"].as_object().unwrap() {
        if name.starts_with("shape_box_") {scene["buffers"][name.replace("shape_box_","test_unit_box_")]=buffer.clone();}
    }
    fn nodes(value:&mut serde_json::Value) {
        for node in value.as_array_mut().into_iter().flatten() {
            if node["geometry"]["std"]=="rounded_box" && node["geometry"]["radius"]==0 {
                let size=node["geometry"]["size"].clone();
                let scale:Vec<_>=(0..3).map(|i|size[i].as_f64().unwrap()*node["scale"][i].as_f64().unwrap_or(1.)).collect();
                node["geometry"]=serde_json::json!("test_unit_box");node["scale"]=serde_json::json!(scale);
            }
            if let Some(children)=node.get_mut("children") {nodes(children);}
        }
    }
    nodes(&mut scene["nodes"]);scene
}
