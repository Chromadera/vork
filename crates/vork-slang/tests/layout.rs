//! Input reflection and packing against checked-in slangc reflection JSON.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vork_slang::{InputLayout, Type};

fn layout() -> InputLayout {
    let reflection: Value =
        serde_json::from_str(include_str!("fixtures/inputs-reflection.json")).unwrap();
    InputLayout::reflect(&reflection).unwrap()
}
fn pack(values: Value) -> anyhow::Result<Vec<u32>> {
    let values: BTreeMap<String, Value> = serde_json::from_value(values).unwrap();
    layout().pack(&values, |_, v| match v.as_str() {
        Some("brick") => Ok(7),
        _ => anyhow::bail!("unknown image"),
    })
}

#[test]
fn reflects_word_layout() {
    let l = layout();
    let fields: Vec<_> = l
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.ty, f.offset))
        .collect();
    assert_eq!(
        fields,
        [
            ("tint", Type::Vector(3), 1),
            ("roughness", Type::Float, 4),
            ("glossy", Type::Bool, 5),
            ("bands", Type::Int, 6),
            ("seed", Type::Uint, 7),
            ("albedo", Type::Image, 8)
        ]
    );
    assert_eq!(l.words, 9);
}

#[test]
fn packs_supplied_inputs_only() {
    let w = pack(json!({"tint":[0.2,0.4,0.8],"glossy":false,"bands":-3,"seed":9,"albedo":"brick"}))
        .unwrap();
    assert_eq!(w[0], 0b111101);
    assert_eq!(
        &w[1..4],
        &[0.2f32.to_bits(), 0.4f32.to_bits(), 0.8f32.to_bits()]
    );
    assert_eq!(
        w[4], 0,
        "unsupplied input keeps its Slang initializer, not a packed value"
    );
    assert_eq!((w[5], w[6] as i32, w[7], w[8]), (0, -3, 9, 7));
}

#[test]
fn rejects_bad_values_by_name() {
    for (values, message) in [
        (json!({"nope":1}), "unknown program input \"nope\""),
        (json!({"tint":[1,2]}), "input tint: expected 3 numbers"),
        (
            json!({"roughness":1e39}),
            "input roughness: outside finite float32",
        ),
        (json!({"roughness":"x"}), "input roughness: expected number"),
        (json!({"glossy":1}), "input glossy: expected bool"),
        (json!({"bands":3000000000u64}), "input bands: outside int32"),
        (json!({"seed":-1}), "input seed: expected unsigned integer"),
        (json!({"albedo":"missing"}), "unknown image"),
    ] {
        let error = format!("{:#}", pack(values).unwrap_err());
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn rejects_unsupported_types() {
    let reflection: Value =
        serde_json::from_str(include_str!("fixtures/unsupported-reflection.json")).unwrap();
    let error = InputLayout::reflect(&reflection).unwrap_err().to_string();
    assert!(error.contains("input basis: unsupported type"), "{error}");
}

#[test]
fn version_matches_pin() {
    let pin: Value = serde_json::from_str(include_str!("../../../dist/slang.json")).unwrap();
    assert_eq!(pin["version"], vork_slang::VERSION);
}

#[test]
fn integral_float_inputs_pack_as_reflected_integers() {
    assert_eq!(pack(json!({"bands":-3.0,"seed":9.0})).unwrap(),pack(json!({"bands":-3,"seed":9})).unwrap());
    assert!(pack(json!({"bands":1.5})).is_err());
    assert!(pack(json!({"seed":4294967296.0})).is_err());
}
