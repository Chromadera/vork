use super::*;
use serde_json::json;

fn render(r: &Renderer, source: &str, color: f32) -> Result<FrameOutput> {
    render_samples(r, source, color, 1)
}
fn render_samples(r: &Renderer, source: &str, color: f32, samples: u32) -> Result<FrameOutput> {
    let scene = serde_json::from_value(
        json!({"version":2,"render":{"resolution":[8,8],"background":[color,color,color],"settings":{"integrator":"raster","samples":samples,"reflections":"environment"},"display":{"program":"display"}},
        "programs":{"display":{"source":source}},"nodes":[{"type":"camera"}]}),
    )?;
    let scene = r.install(scene, Path::new("."))?;
    let mut frame = scene.create_frame_slot(r, ReadbackPrecision::Float)?;
    frame.submit_outputs(
        &scene,
        r,
        0.,
        ReadbackOutputs {
            color: true,
            float: FloatOutputs {
                color: true,
                ..Default::default()
            },
            ..Default::default()
        },
    )?;
    frame.collect(r)
}
#[test]
fn display_program_preserves_sharp_thresholds_and_unclamped_hdr_inputs() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let source = "struct Inputs {}; float3 display(DisplayContext ctx, Inputs i) {return float3(ctx.color.x > .181 ? 1. : 0.);}";
    for (color, expected) in [(0.18, 0), (0.19, 255)] {
        let result = render(&r, source, color)?;
        assert_eq!(&result.rgba[..3], &[expected; 3]);
        assert!((result.linear_rgba.as_ref().unwrap()[0][0] - color).abs() < 0.001);
    }
    let result = render(
        &r,
        "struct Inputs {}; float3 display(DisplayContext ctx, Inputs i) {return ctx.color/100000.;}",
        50000.,
    )?;
    assert!(
        (i32::from(result.rgba[0]) - 128).abs() <= 1,
        "{:?}",
        &result.rgba[..4]
    );
    assert!(result.linear_rgba.as_ref().unwrap()[0][0] > 49000.);
    Ok(())
}
#[test]
fn display_program_nonfinite_result_is_a_frame_error() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let result = render(
        &r,
        "struct Inputs {}; float3 display(DisplayContext ctx, Inputs i) {return float3(1./(ctx.color.x-ctx.color.x));}",
        0.18,
    );
    let error = format!(
        "{:#}",
        result
            .err()
            .context("nonfinite display program succeeded")?
    );
    assert!(
        error.contains("display program returned a non-finite"),
        "{error}"
    );
    Ok(())
}

#[test]
fn display_program_runs_only_after_final_sample() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    // Scratch is initially zero. Evaluating before resolve's final sample would
    // report a non-finite value, even though the actual scene color is positive.
    let source =
        "struct Inputs {}; float3 display(DisplayContext ctx, Inputs i) {return .09/ctx.color;}";
    let single = render_samples(&r, source, 0.18, 1)?;
    let accumulated = render_samples(&r, source, 0.18, 4)?;
    assert_eq!(single.rgba, accumulated.rgba);
    assert!((i32::from(accumulated.rgba[0]) - 128).abs() <= 1);
    Ok(())
}
