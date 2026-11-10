use super::*;

#[test]
fn hidden_world_medium_has_no_absorption_or_emission() -> Result<()> {
    let Some(renderer) = crate::test_renderer()? else { return Ok(()); };
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // Point into the dense lower exterior: constant-emission height fog diverges
    // upward. Keep the bounded volume and capture screen centered on this view.
    for (falloff,capture,bounded) in [(0.,false,false),(0.2,false,false),(0.,true,false),(0.,false,true),(0.2,false,true)] {
        eprintln!("world visibility: falloff={falloff}, capture={capture}, bounded={bounded}");
        let mut input = serde_json::json!({"version":2,
            "render":{"resolution":[8,8],"background":[0.4,0.4,0.4],"settings":{"samples":1}},
            "materials":{"fog":{"program":"std/fog","inputs":{"density":0.5,"scattering":[0,0,0],"absorption":[1,1,1],"emission":[0.3,0,0],"height_falloff":falloff}},"camera_settings":{"program":"std/camera","inputs":{"clip_end":10}}},
            "nodes":[{"type":"camera","name":"cam","target":[0,-0.6,-0.8],"material":"camera_settings"},
                {"type":"node","name":"parent","children":[{"type":"light","shape":"environment","medium":"fog","color":[0,0,0]}]}],
            "animation":[{"tracks":[{"path":["nodes","parent","visible"],"keys":[[0,false],[1,true]]}]}]
        });
        if bounded {
            let fixture: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/analytic.json"))?;
            input["buffers"] = fixture["buffers"].clone();
            input["geometry"] = fixture["geometry"].clone();
            input["nodes"].as_array_mut().unwrap().push(serde_json::json!({"type":"mesh","geometry":"shape_box","materials":["fog"],"translation":[0,-1.8,-2.4]}));
        }
        if capture {
            input["buffers"] = serde_json::json!({
                "p":{"type":"vec3","data":[[-1,-1.4,-0.2],[1,-1.4,-0.2],[1,0.2,-1.4],[-1,0.2,-1.4]]},
                "uv":{"type":"vec2","data":[[0,0],[1,0],[1,1],[0,1]]},
                "i":{"type":"u32","data":[0,1,2,0,2,3]}});
            input["geometry"] = serde_json::json!({"screen":{"attributes":{"POSITION":"p","TEXCOORD_0":"uv"},"indices":"i"}});
            input["materials"]["display"] = serde_json::json!({"inputs":{"base_weight":0,"emission_color":{"image":"feed"},"emission_luminance":1}});
            input["images"] = serde_json::json!({"feed":{"render":{"camera":"cam","root":"parent","resolution":[8,8],"background":[0.4,0.4,0.4]}}});
            input["nodes"].as_array_mut().unwrap().push(serde_json::json!({"type":"mesh","name":"screen","geometry":"screen","materials":["display"]}));
            input["render"]["root"] = "screen".into();

        }
        let mut installed = renderer.install(vork_scene::v2::from_test_value(input.clone())?,&base)?;
        let mut frame = installed.create_frame_slot(&renderer,Default::default())?;
        let mut images = Vec::new();
        for time in [0.,1.,0.] {
            images.push(installed.render_frame(&renderer, &mut frame, time)?.rgba);
        }
        let mut clear = input;
        clear["nodes"][1]["children"][0].as_object_mut().unwrap().remove("medium");
        let mut installed = renderer.install(vork_scene::v2::from_test_value(clear)?,&base)?;
        let mut frame = installed.create_frame_slot(&renderer,Default::default())?;
        let clear = installed.render_frame(&renderer, &mut frame, 0.)?.rgba;
        assert_eq!(images[0],clear,"hidden world medium at falloff {falloff}");
        assert_eq!(images[2],clear,"backward sample at falloff {falloff}");
        assert_ne!(images[1],clear,"visible world medium must affect pixels");
    }
    Ok(())
}

/// BB105: a hidden middle TLAS slot must not corrupt the later live slot on CPU.
#[test]
fn hidden_geometry_matches_deleted_geometry() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { return Ok(()); };
    let input: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/hidden-geometry-bb105.json"))?;
    let render = |input: serde_json::Value, times: &[f32]| -> Result<Vec<Vec<[f32; 4]>>> {
        let mut scene = crate::test_scene(input, Path::new("."))?;
        scene.render.settings = vork_scene::Profile::load("std/path")?.resolve(
            serde_json::json!({"samples": 1, "denoise": false, "seed": 0}).as_object().unwrap())?;
        let installed = r.install(scene, Path::new("."))?;
        let mut slot = installed.create_frame_slot(&r, ReadbackPrecision::Float)?;
        let mut frames = Vec::new();
        for &time in times {
            slot.submit_outputs(&installed, &r, time, ReadbackOutputs { float: FloatOutputs { color: true, ..Default::default() }, ..Default::default() })?;
            let pixels = slot.collect(&r)?.linear_rgba.context("BB105 linear colour")?;
            assert!(pixels.iter().flatten().all(|c| c.is_finite()));
            frames.push(pixels);
        }
        Ok(frames)
    };
    let hidden = render(input.clone(), &[0.])?;
    let mut visible = input.clone();
    visible["nodes"][1]["visible"] = true.into();
    let visible = render(visible, &[1.])?;
    assert_eq!(visible[0].len(), hidden[0].len());
    let mut deleted = input.clone();
    deleted["nodes"].as_array_mut().unwrap().remove(1);
    let deleted = render(deleted, &[0.])?;
    assert!(hidden == deleted, "hidden and deleted geometry must produce identical linear pixels");
    // Reuse a frame slot through visibility changes and a backward seek. The
    // software placeholder must restore the real BLAS when visibility returns.
    let mut animated = input;
    // The repro's fox is behind the screen. Bring it in front for an observable
    // reactivation check, while retaining the unmodified three controls above.
    animated["nodes"][1]["translation"][2] = 1.into();
    animated["nodes"][1]["visible"] = true.into();
    let visible_front = render(animated.clone(), &[1.])?;
    assert!(visible_front[0] != hidden[0], "reactivated geometry must affect pixels");
    animated["animation"] = serde_json::json!([{"tracks":[{"path":["nodes","fox","visible"],"keys":[[0,false],[1,true]]}]}]);
    let animated = render(animated, &[0., 1., 0.])?;
    assert!(animated[0] == hidden[0], "initial hidden animation frame");
    assert!(animated[1] == visible_front[0], "reactivated geometry");
    assert!(animated[2] == hidden[0], "backward seek to hidden geometry");
    r.errors.check()?;
    Ok(())
}
