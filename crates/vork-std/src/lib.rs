//! Unstable, internal: all Rust APIs in this crate are free to change.
//!
//! Language-independent JSON fragments from the same implementation as @usevoyager/vork.
//! The embedded engine has no host bindings, filesystem or network access.
mod expand;
pub use expand::{expand_scene, load_scene};
pub mod environment;
mod sky;
mod extrude;
mod text;

use anyhow::{Context, Result};
#[cfg(test)]
use serde_json::Value;

pub const HELP: &str = include_str!("help.txt");

// Native counterpart of validate.ts clampRadii: one proportional scale satisfies
// all edge constraints. Callers validate finite, nonnegative radii first.
fn clamp_radii(radii: &[f32], edges: &[(f32, &[usize])]) -> Vec<f32> {
    let maximum = radii.iter().copied().fold(0f32, f32::max);
    if maximum == 0. { return radii.to_vec(); }
    let mut fitted = maximum;
    for (length, corners) in edges {
        let sum: f32 = corners.iter().map(|&i| radii[i] / maximum).sum();
        if sum > 0. { fitted = fitted.min(length / sum); }
    }
    if fitted == maximum { radii.to_vec() }
    else { radii.iter().map(|r| (r / maximum) * fitted).collect() }
}

pub fn evaluate(function: &str, argument: &str) -> Result<String> {
    match function {
        "expand" => {
            let mut scene = serde_json::from_str(argument)?;
            expand_scene(&mut scene, &std::env::current_dir()?)?;
            return Ok(serde_json::to_string(&scene)?);
        }
        "sky" => return sky::evaluate(argument),
        "text" => return text::evaluate(argument),
        "extrude" => return extrude::evaluate(argument),
        _ => {}
    }
    let runtime = rquickjs::Runtime::new().context("initialize standard-library engine")?;
    let context =
        rquickjs::Context::full(&runtime).context("initialize standard-library context")?;
    let script = format!(
        "{}\nJSON.stringify(stdEvaluate({},JSON.parse({})), (key,value) => {{ if (typeof value === 'number' && !Number.isFinite(value)) throw new Error('non-finite standard-library result at '+key); return value; }})",
        include_str!("sdk.js"),
        serde_json::to_string(function)?,
        serde_json::to_string(argument)?
    );
    context.with(|ctx| {
        ctx.eval::<String, _>(script).map_err(|error| {
            let exception = ctx.catch();
            let message = exception
                .as_exception()
                .and_then(|e| e.message())
                .unwrap_or_else(|| error.to_string());
            anyhow::anyhow!("standard-library function failed: {message}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_names_default_to_builder() {
        for (builder, options) in [
            ("box", serde_json::json!({})),
            ("sphere", serde_json::json!({})),
            ("cylinder", serde_json::json!({})),
            ("cone", serde_json::json!({})),
            ("torus", serde_json::json!({})),
            ("capsule", serde_json::json!({})),
            ("rounded_box", serde_json::json!({})),
            ("grid", serde_json::json!({})),
            ("revolve", serde_json::json!({"profile":[[0,0],[1,0],[1,1],[0,1]]})),
            ("extrude", serde_json::json!({"outline":[[0,0],[1,0],[0,1]]})),
            ("text", serde_json::json!({"body":"x","font":concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/LiberationSans-Regular.ttf")})),
        ] {
            let implicit: Value = serde_json::from_str(&evaluate(builder, &options.to_string()).unwrap()).unwrap();
            let mut explicit = options.clone();
            explicit["name"] = builder.into();
            let expected: Value = serde_json::from_str(&evaluate(builder, &explicit.to_string()).unwrap()).unwrap();
            assert_eq!(implicit, expected, "{builder}");
            assert_eq!(implicit["geometry"][builder]["attributes"]["POSITION"], format!("{builder}_position"));
            assert_eq!(implicit["buffers"].as_object().unwrap().len(), 4);
            for suffix in ["position", "normal", "TEXCOORD_0", "indices"] {
                assert!(implicit["buffers"].get(format!("{builder}_{suffix}")).is_some());
            }
            explicit["name"] = "custom".into();
            let named: Value = serde_json::from_str(&evaluate(builder, &explicit.to_string()).unwrap()).unwrap();
            assert_eq!(named["geometry"]["custom"]["attributes"]["POSITION"], "custom_position");
            for name in [serde_json::json!(""), serde_json::json!("bad/name"), Value::Null] {
                explicit["name"] = name;
                assert!(evaluate(builder, &explicit.to_string()).is_err(), "{builder}");
            }
        }
    }

    #[test]
    fn oversized_radii_and_repeated_profiles_match_canonical_geometry() {
        for (builder, requested, canonical) in [
            ("rounded_box", r#"{"size":[0.18,0.18,0.012],"radius":0.02}"#,
                r#"{"size":[0.18,0.18,0.012],"radius":0.006}"#),
            ("rounded_box", r#"{"size":[1,0,2],"radius":1}"#,
                r#"{"size":[1,0,2],"radius":0}"#),
            ("capsule", r#"{"height":1,"radius":2}"#, r#"{"height":1,"radius":0.5}"#),
            ("torus", r#"{"radius":1,"tube":2}"#, r#"{"radius":1,"tube":1}"#),
            ("revolve", r#"{"profile":[[0,0],[0,0],[1,0],[1,0],[1,1],[0,1],[0,1]]}"#,
                r#"{"profile":[[0,0],[1,0],[1,1],[0,1]]}"#),
        ] {
            assert_eq!(evaluate(builder, requested).unwrap(), evaluate(builder, canonical).unwrap(), "{builder}");
        }
        assert!(evaluate("revolve", r#"{"profile":[[1,0],[1,0]]}"#).is_err());
        assert!(evaluate("rounded_box", r#"{"radius":-1}"#).is_err());
    }

    #[test]
    fn geometry_and_errors() {
        let fragment: Value =
            serde_json::from_str(&evaluate("box", r#"{"name":"cube"}"#).unwrap()).unwrap();
        assert_eq!(
            fragment["buffers"]["cube_position"]["data"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        for (function, argument) in [
            ("sphere", r#"{"name":"s","radius":-1}"#),
            ("sphere", r#"{"name":"s","segment":8}"#),
            ("box", r#"{"name":"b","radius":0.2}"#),
            ("box", r#"{"name":"b","size":[1,2]}"#),
            ("box", "not JSON"),
        ] {
            assert!(
                evaluate(function, argument).is_err(),
                "{function} {argument}"
            );
        }
        for input in [
            r#"{"name":"x","font":"font.ttf"}"#,
            r#"{"name":"x","body":"x"}"#,
        ] {
            assert!(
                evaluate("text", input)
                    .unwrap_err()
                    .to_string()
                    .contains("missing field")
            );
        }
    }
}
