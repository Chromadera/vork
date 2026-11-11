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
    crate::test_display::assert_display_eq(&single.rgba, &accumulated.rgba);
    let wrong = render_samples(&r, "struct Inputs {}; float3 display(DisplayContext ctx, Inputs i) {return ctx.color;}", 0.18, 4)?;
    // Bypassing the transform preserves linear readback but must fail here.
    crate::test_linear::assert_linear_eq(wrong.linear_rgba.as_ref().unwrap(), accumulated.linear_rgba.as_ref().unwrap());
    crate::test_display::assert_display_ne(&wrong.rgba, &accumulated.rgba);
    assert!((i32::from(accumulated.rgba[0]) - 128).abs() <= 1);
    Ok(())
}

#[test]
fn cloth_rest_edge_filter_keeps_small_crossing_edges_out_of_contacts() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { return Ok(()); };
    // Execute the shipped Slang functions, rather than a Rust copy of their
    // arithmetic. Display output is only a compact GPU numeric receipt.
    let library = include_str!("../../../libraries/cloth/self.slang");
    let segments = library.split_once("void cloth_segments(").unwrap().1
        .split_once("void cloth_register(").unwrap().0;
    let rest_edges = library.split_once("bool cloth_rest_edges(").unwrap().1
        .split_once("bool cloth_self_boxes(").unwrap().0;
    let source = format!(r#"
        static const float SELF_MARGIN=3e-3;
        void cloth_segments({segments}
        float3 rest_position_load(uint i) {{
            if(i==0) return float3(-4e-5,0,0);
            if(i==1) return float3(4e-5,0,0);
            if(i==2) return float3(0,-4e-5,0);
            return float3(0,4e-5,0);
        }}
        bool cloth_rest_edges({rest_edges}
        struct Inputs {{}};
        float3 display(DisplayContext ctx, Inputs inputs) {{
            bool ok=cloth_rest_edges(uint4(0,1,2,3),6e-6);
            ok=ok&&!cloth_rest_edges(uint4(0,1,2,3),0);
            for(uint i=0;i<3;i++) {{
                float scale=i==0?8e-5:(i==1?1.:100.);
                float3 p=float3(-.5,0,0)*scale,q=-p;
                float3 a=float3(0,-.5,0)*scale,b=-a;
                float s,t;cloth_segments(p,q,a,b,s,t);
                ok=ok&&abs(s-.5)<1e-5&&abs(t-.5)<1e-5;
                // Parallel, separated perpendicular and zero-length edges.
                a=float3(-.5,1,0)*scale;b=float3(.5,1,0)*scale;
                cloth_segments(p,q,a,b,s,t);
                ok=ok&&abs(length(lerp(p,q,s)-lerp(a,b,t))/scale-1)<1e-5;
                a=float3(0,-.5,1)*scale;b=float3(0,.5,1)*scale;
                cloth_segments(p,q,a,b,s,t);
                ok=ok&&abs(length(lerp(p,q,s)-lerp(a,b,t))/scale-1)<1e-5;
                cloth_segments(float3(0),float3(0),a,b,s,t);
                ok=ok&&s==0&&abs(t-.5)<1e-5;
                cloth_segments(a,b,float3(0),float3(0),s,t);
                ok=ok&&t==0&&abs(s-.5)<1e-5;
                cloth_segments(float3(0),float3(0),float3(0),float3(0),s,t);
                ok=ok&&s==0&&t==0;
                cloth_segments(float3(0),float3(1e-10,0,0)*scale,a,b,s,t);
                ok=ok&&s==0&&abs(t-.5)<1e-5;
            }}
            return ok?float3(1):float3(0);
        }}
    "#);
    let result = render(&r, &source, 0.)?;
    assert!(result.rgba.chunks_exact(4).all(|pixel| pixel[..3] == [255; 3]),
        "cloth closest-point/rest-edge numeric receipt failed: {:?}", &result.rgba[..4]);
    Ok(())
}
