//! Test bridge for SDK fragments: use the kernel validator rather than duplicate it in JS.
use std::io::Read;
fn main() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    for (index, value) in serde_json::from_str::<Vec<serde_json::Value>>(&input)?
        .into_iter()
        .enumerate()
    {
        let scene: vork_scene::v2::Scene = serde_json::from_value(value)?;
        scene
            .validate()
            .map_err(|e| anyhow::anyhow!("SDK fixture {index}: {e:#}"))?;
    }
    Ok(())
}
