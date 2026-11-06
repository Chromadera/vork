use super::*;
use serde_json::{Value, json};

fn scene(far: f32) -> Value {
    json!({"version":2,"render":{"resolution":[8,8],"background":[1,1,1],"settings":{"integrator":"raster","samples":1,"reflections":"environment","fast_gi":null},"display":{"program":"std/standard","inputs":{"dither_intensity":0}}},
        "materials":{"fog":{"program":"std/fog","inputs":{"absorption":[0.1,0.2,0.4],"scattering":[0,0,0],"emission":[0.03,0.04,0.04]}}},
        "nodes":[{"type":"camera","name":"camera","material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":2,"clip_start":0.01,"clip_end":far}}},
            {"type":"light","shape":"environment","energy":0,"medium":"fog"}]})
}
fn render(r: &Renderer, mut value: Value) -> Result<Vec<[f32; 4]>> {
    let settings=value["render"].as_object_mut().unwrap().remove("settings").unwrap();
    let mut scene=crate::test_scene(value.clone(), std::path::Path::new("."))?;
    scene.render.settings=serde_json::from_value(settings)?;
    let mut installed = r.install(scene, Path::new("."))?;
    let mut slot = installed.create_frame_slot(r, ReadbackPrecision::Float)?;
    installed.render_frame(r,&mut slot,0.)?.linear_rgba.context("linear readback")
}
fn cover(value: &mut Value) {
    value["materials"]["black"] = json!({"inputs":{"base_weight":0,"specular_weight":0}});
    value["buffers"] =
        json!({"p":{"type":"vec3","data":[[-10,-10,-0.25],[10,-10,-0.25],[0,10,-0.25]]}});
    value["geometry"] = json!({"g":{"attributes":{"POSITION":"p"}}});
    value["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"mesh","geometry":"g","materials":["black"]}));
}
#[test]
fn scattering_only_fog_at_22_metres_has_no_hidden_absorption() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { return Ok(()); };
    for absorption in [None, Some([0.02, 0.04, 0.08])] {
        let mut value = scene(50.);
        value["materials"]["fog"]["inputs"] = json!({"scattering":[0.01,0.01,0.01]});
        if let Some(coefficients) = absorption {
            value["materials"]["fog"]["inputs"]["absorption"] = json!(coefficients);
        }
        cover(&mut value);
        value["nodes"][0]["material"]["inputs"]["ortho_scale"] = json!(0.0001);
        value["buffers"]["p"]["data"] = json!([[-0.001,-0.001,-22],[0.001,-0.001,-22],[0.001,0.001,-22],
            [-0.001,-0.001,-22],[0.001,0.001,-22],[-0.001,0.001,-22]]);
        // Mesh emission now lights fog. A millimetre receiver and narrow view
        // keep its physical in-scatter below the unchanged extinction tolerance.
        // Camera radiance remains one, over the same 22-metre segment.
        value["materials"]["black"]["inputs"]["emission_color"] = json!([1,1,1]);
        value["materials"]["black"]["inputs"]["emission_luminance"] = json!(1);
        for pixel in render(&r, value)? {
            for c in 0..3 {
                let expected = (-22.0_f64 * (0.01 + absorption.unwrap_or([0.;3])[c])).exp();
                assert!((f64::from(pixel[c]) - expected).abs() < 6e-4,
                    "absorption {absorption:?}, channel {c}: {} != {expected}", pixel[c]);
            }
        }
    }
    Ok(())
}
#[test]
fn unbounded_world_background_and_finite_receivers() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    for far in [0.5, 50., 500.] {
        let mut value = scene(far);
        for pixel in render(&r, value.clone())? {
            for (actual, expected) in pixel[..3].iter().zip([0.3, 0.2, 0.1]) {
                // Scene-linear composition retains half-float colour before readback.
                assert!((actual - expected).abs() < 3e-4, "far {far}: {pixel:?}");
            }
        }
        cover(&mut value);
        let expected = [
            0.3 * (1. - (-0.1_f32 * 0.25).exp()),
            0.2 * (1. - (-0.2_f32 * 0.25).exp()),
            0.1 * (1. - (-0.4_f32 * 0.25).exp()),
        ];
        for pixel in render(&r, value)? {
            for (actual, expected) in pixel[..3].iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 2e-5,
                    "finite receiver far {far}: {pixel:?}, expected {expected}"
                );
            }
        }
    }
    Ok(())
}
#[test]
fn divergent_world_emission_errors_only_for_uncovered_background() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let mut value = scene(1.);
    value["materials"]["fog"]["inputs"]["absorption"] = json!([0, 0, 0]);
    assert!(format!("{:#}", render(&r, value.clone()).unwrap_err()).contains("zero-extinction"));
    cover(&mut value);
    for pixel in render(&r, value)? {
        assert!(
            (pixel[0] - 0.03 * 0.25).abs() < 2e-5,
            "finite emissive segment: {pixel:?}"
        );
    }
    Ok(())
}
#[test]
fn varying_world_coefficients_beyond_far_plane_are_integrated() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let mut value = scene(1.);
    value["programs"] = json!({"distant":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext ctx, Inputs i) {MediumCoefficients m; m.absorption=float3(ctx.position.z < -10. ? .2 : 0.); m.scattering=float3(0.); m.emission=m.absorption*.25; m.anisotropy=0.; return m;}"}});
    value["materials"]["fog"] = json!({"program":"distant"});
    for pixel in render(&r, value)? {
        for actual in &pixel[..3] {
            assert!(
                (actual - 0.25).abs() < 2e-4,
                "distant world ignored: {pixel:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn unbounded_spectral_medium_extinguishes_distant_light_and_retains_local_emission() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    for far in [0.5, 50.] {
      // World connections extend to infinity: nonzero extinction removes their
      // incident radiance. Local emission still has the exact J / sigma_t tail.
      for (emission, expected) in [([0.,0.,0.],[0.,0.,0.]),
          ([0.06,0.16,0.4],[0.15,0.4*0.4/0.6,0.5])] {
        let mut value = scene(far);
        value["materials"]["fog"]["inputs"] =
            json!({"absorption":[0.1,0.2,0.3],"scattering":[0.3,0.4,0.5],"emission":emission});
        value["nodes"][1]["energy"] = json!(1);
        value["nodes"][1]["color"] = json!([0.2, 0.4, 0.8]);
        for pixel in render(&r, value)? {
            for (actual, expected) in pixel[..3].iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 4e-4,
                    "spectral medium far {far}, emission {emission:?}: {pixel:?}"
                );
            }
        }
      }
    }
    Ok(())
}

#[test]
fn varying_zero_extinction_world_emission_is_refused() -> Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let mut value = scene(1.);
    value["programs"] = json!({"distant":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext ctx, Inputs i) {MediumCoefficients m; m.absorption=float3(0.); m.scattering=float3(0.); m.emission=float3(ctx.position.z < -10. ? .01 : 0.); m.anisotropy=0.; return m;}"}});
    value["materials"]["fog"] = json!({"program":"distant"});
    let error = format!("{:#}", render(&r, value.clone()).unwrap_err());
    assert!(
        error.contains("zero-extinction"),
        "{error}"
    );
    cover(&mut value);
    for pixel in render(&r, value)? {
        assert_eq!(&pixel[..3], &[0., 0., 0.]);
    }
    Ok(())
}

#[test]
fn emissive_height_fog_finite_optical_tail_reports_divergence() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for height in [0.,100.] {
        let mut value=scene(0.01);
        value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
        value["nodes"][0]["translation"]=json!([0,height,0]);
        value["nodes"][0]["target"]=json!([0,height+1.,0]);
        value["materials"]["fog"]["inputs"]=json!({"absorption":[1,2,4],"scattering":[0,0,0],
            "height_falloff":1,"emission":[0.001,0.001,0.001]});
        let error=format!("{:#}",render(&r,value).unwrap_err());
        assert!(error.contains("zero-extinction"),"{error}");
    }
    Ok(())
}

#[test]
fn emissive_exponential_height_tail_matches_analytic_integral() -> Result<()> {
    let Some(mut r)=crate::test_renderer()? else {return Ok(());};
    // sigma=.2 exp(-.5t), clamped at exp(-4), with constant emission .001.
    // The finite integral is exp(-x)/k [Ei(x)-Ei(x exp(-4))], evaluated
    // by the convergent analytic series for the Ei difference.
    let (x,y)=(0.4f64,0.4f64*(-4f64).exp());
    let mut difference=4.;let mut factorial=1.;
    for n in 1..40 {factorial*=n as f64;difference+=(x.powi(n)-y.powi(n))/(n as f64*factorial);}
    let expected=0.001*((-x).exp()/0.5*difference+(-(x-y)).exp()/(0.2*(-4f64).exp()));
    for far in [0.5,3.,30.] {
        let mut value=scene(far);
        value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
        value["nodes"][0]["rotation"]=json!([0.7071067811865476,0,0,0.7071067811865476]);
        r.reference_medium_grid=Some((8,256,0.8));
        value["programs"]=json!({"height":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m={};m.absorption=float3(.2*exp(clamp(-.5*c.position.y,-4.,4.)));m.emission=float3(.001);return m;}"}});
        value["materials"]["fog"]=json!({"program":"height"});
        for pixel in render(&r,value)? {
            for actual in &pixel[..3] {assert!((f64::from(*actual)-expected).abs()<expected*0.002,"far {far}: {pixel:?}, analytic {expected}");}
        }
    }
    Ok(())
}

#[test]
fn varying_emissive_world_without_tail_proof_is_refused_with_positive_extinction() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(1.);
    value["programs"]=json!({"varying":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m={};m.absorption=float3(.2+.1*sin(c.position.z));m.emission=float3(.01);return m;}"}});
    value["materials"]["fog"]=json!({"program":"varying"});
    let error=format!("{:#}",render(&r,value.clone()).unwrap_err());
    assert!(error.contains("varying emission without asymptotic closure is unsupported"),"{error}");
    cover(&mut value);
    assert!(render(&r,value)?.iter().all(|pixel|pixel[..3].iter().all(|c|c.is_finite() && *c>0.)));
    Ok(())
}

#[test]
fn height_fog_upward_transmission_matches_analytic_optical_depth() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for (height,up,falloff,scale) in [(0.,1.,1.,1.),(1.6,0.07,0.05,0.0015),
        (10.,0.6,0.1,0.2),(100.,1.,1.,1.),(-2.,0.6,0.5,0.1),(0.,1.,1.,0.)] {
        for far in [0.001,0.01] {
            let mut value=scene(far);
            value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
            value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.00001);
            value["nodes"][0]["translation"]=json!([0,height,0]);
            value["nodes"][0]["target"]=json!([0,height+up,-(1f64-up*up).sqrt()]);
            value["materials"]["fog"]["inputs"]=json!({"absorption":[0,scale,2.*scale],
                "scattering":[0,0,0],"height_falloff":falloff});
            for pixel in render(&r,value)? {
                for (actual,sigma) in pixel[..3].iter().zip([0.,scale,2.*scale]) {
                    let expected=(-sigma*(-falloff*height).exp()/(falloff*up)).exp();
                    assert!((f64::from(*actual)-expected).abs()<0.002,
                        "height {height}, up {up}, falloff {falloff}, far {far}: {pixel:?}, expected {expected}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn varying_world_with_default_zero_emission_remains_supported() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(1.);
    value["render"]["background"]=json!([0,0,0]);
    value["programs"]=json!({"unlit":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m;m.absorption=float3(.2+.1*sin(c.position.z));return m;}"}});
    value["materials"]["fog"]=json!({"program":"unlit"});
    for pixel in render(&r,value)? {assert_eq!(&pixel[..3],&[0.,0.,0.]);}
    Ok(())
}

#[test]
fn optically_thick_height_fog_matches_analytic_emission() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    // Downward sigma(s)=a exp(beta*s), C=a/beta=100. The clamped
    // remainder is negligible. exp(C) E1(C) has an alternating expansion
    // whose error is bounded by the next term; 20 terms suffice here.
    let mut sum=1f64;let mut term=1f64;
    for n in 1..20 {term*=-(n as f64)/100.;sum+=term;}
    let expected=0.01*sum;
    for far in [0.001,0.01] {
        let mut value=scene(far);
        value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
        value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.0001);
        value["nodes"][0]["target"]=json!([0,-0.6,-0.8]);
        value["materials"]["fog"]["inputs"]=json!({"absorption":[0.6,0.6,0.6],"scattering":[0,0,0],"height_falloff":0.01,"emission":[0.006,0.006,0.006]});
        for pixel in render(&r,value)? {
            for actual in &pixel[..3] {assert!((f64::from(*actual)-expected).abs()<expected*0.002+2e-6,"far {far}: {pixel:?}, analytic {expected}");}
        }
    }
    Ok(())
}

#[test]
fn proven_zero_extinction_emitting_region_has_a_finite_tail() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for far in [0.001,0.01] {
        let mut value=scene(far);
        value["render"]["background"]=json!([0,0,0]);
        value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
        value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.0001);
        value["nodes"][0]["target"]=json!([0,0.6,-0.8]);
        value["programs"]=json!({"layer":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m;m.emission=float3(c.position.y<1. ? .25 : 0.);return m;}"}});
        value["materials"]["fog"]=json!({"program":"layer"});
        for pixel in render(&r,value)? {
            for actual in &pixel[..3] {assert!((actual-0.25/0.6).abs()<0.001,"far {far}: {pixel:?}");}
        }
    }
    Ok(())
}

#[test]
fn thin_height_fog_preserves_direct_sun_disc() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(0.01);
    value["nodes"][0]["material"]["inputs"]["clip_start"]=json!(0.00001);
    value["render"]["background"]=json!("environment");
    value["nodes"][0]["translation"]=json!([0,1.6,0]);
    value["nodes"][0]["target"]=json!([0,1.67,-(1f64-0.07*0.07).sqrt()]);
    value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.00001);
    // Isolate direct disc transmission from atmospheric radiance and in-scatter.
    value["nodes"][1]["color"]=json!([0,0,0]);
    let axis=Vec3::new(0.,0.07,-(1f32-0.07*0.07).sqrt());
    value["nodes"].as_array_mut().unwrap().push(json!({"type":"light","shape":"sun","rotation":glam::Quat::from_rotation_arc(Vec3::NEG_Z,-axis).to_array(),"angle":0.04,"energy":std::f32::consts::PI*0.02f32.sin().powi(2)}));
    value["materials"]["fog"]["inputs"]=json!({"absorption":[0.0015,0.0015,0.0015],
        "scattering":[0,0,0],"height_falloff":0.05});
    let expected=(-0.0015f64*(-0.05f64*1.6).exp()/(0.05*0.07)).exp();
    for pixel in render(&r,value)? {
        for actual in &pixel[..3] {assert!((f64::from(*actual)-expected).abs()<0.003,
            "direct disc: {pixel:?}, expected {expected}");}
    }
    Ok(())
}

#[test]
fn disjoint_medium_intervals_complete_beyond_ray_depth() -> Result<()> {
    let Some(mut r) = crate::test_renderer()? else {return Ok(());};
    let mut value=scene(100.);
    value["materials"]["fog"]["inputs"]=json!({"absorption":[1,1,1],"scattering":[0,0,0]});
    value["nodes"].as_array_mut().unwrap().truncate(1);
    // All forty crossings lie in one froxel. A traversal budget used to discard
    // the suffix after sixteen intervals, despite a valid receiver at infinity.
    r.reference_medium_grid=Some((8,1,0.8));
    for i in 0..20 {
        value["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh",
            "translation":[0,0,-1.-i as f32*0.2],
            "geometry":{"std":"box","size":[10,10,0.1]},"materials":["fog"]}));
    }
    for far in [0.5,100.] {
        value["nodes"][0]["material"]["inputs"]["clip_end"]=json!(far);
        {
            for pixel in render(&r,value.clone())? {
                assert!((pixel[0]-(-2_f32).exp()).abs()<0.001,"far {far}: {pixel:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn directional_extinction_uses_infinity_not_camera_far() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for far in [0.1,1.,100.] {
        let mut value=scene(far);
        value["render"]["background"]=json!([0,0,0]);
        value["materials"]["fog"]["inputs"]=json!({"absorption":[0,0,0],"scattering":[0.1,0.1,0.1]});
        value["nodes"].as_array_mut().unwrap().push(json!({"type":"light","shape":"sun","energy":10}));
        // A sun at infinity has zero transmission through a homogeneous world.
        // Changing the camera clipping plane cannot make that connection finite.
        for pixel in render(&r,value)? {assert!(pixel[..3].iter().all(|x|x.abs()<2e-5),"far {far}: {pixel:?}");}
    }
    Ok(())
}

#[test]
fn height_fog_horizontal_and_descending_infinity_never_become_vacuum() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for height in [79.,80.,81.,100.] {
        for up in [0.,-0.1] {
            let mut value=scene(0.5);
            value["nodes"][0]["translation"]=json!([0,height,0]);
            value["nodes"][0]["target"]=json!([0,height+up,-1.]);
            value["materials"]["fog"]["inputs"]=json!({"absorption":[0.,1.,2.],"scattering":[0,0,0],"height_falloff":1.});
            for pixel in render(&r,value)? {
                assert!((pixel[0]-1.).abs()<0.001 && pixel[1]<0.001 && pixel[2]<0.001,"height {height}, up {up}: {pixel:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn finite_height_fog_receiver_matches_analytic_rising_and_descending_segments() -> Result<()> {
    let Some(mut r)=crate::test_renderer()? else {return Ok(());};
    for up in [-0.6_f64,0.,0.6] {
        let mut value=scene(1.);
        r.reference_medium_grid=Some((8,1,0.8));
        value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.000001);
        value["nodes"][0]["target"]=json!([0.,up,-(1.-up*up).sqrt()]);
        value["materials"]["fog"]["inputs"]=json!({"absorption":[0.,1.,2.],"scattering":[0,0,0],"height_falloff":4.});
        cover(&mut value);
        value["materials"]["black"]["inputs"]["emission_luminance"]=json!(1.);
        value["materials"]["black"]["inputs"]["emission_color"]=json!([1,1,1]);
        let length=0.25/(1.-up*up).sqrt();let slope=-4.*up;
        let depth=if slope==0. {length} else {(slope*length).exp_m1()/slope};
        // Keep every receiver vertex at the same view depth. Clipping a huge
        // tilted world-Z triangle at this magnification loses pixel coverage.
        let cosine=(1.-up*up).sqrt();let h=0.01;
        let corner=|x:f64,y:f64|json!([x,up*length+y*cosine,-0.25+y*up]);
        let a=corner(-h,-h);let b=corner(h,-h);let c=corner(h,h);let d=corner(-h,h);
        value["buffers"]["p"]["data"]=json!([a,b,c,a,c,d]);
        for domain in ["world","off_ray_boundary","bounded"] {
            let mut scene=value.clone();
            if domain!="world" {
                scene["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh",
                    "translation":if domain=="bounded" {json!([0,0,0])} else {json!([100,0,0])},
                    "geometry":{"std":"box","size":[10,10,10]},"materials":["fog"]}));
            }
            if domain=="bounded" {scene["nodes"][1].as_object_mut().unwrap().remove("medium");}
            for pixel in render(&r,scene)? {
                for (channel,sigma) in [0_f64,1.,2.].into_iter().enumerate() {
                    let expected=(-sigma*depth).exp();
                    assert!((f64::from(pixel[channel])-expected).abs()<0.001,"{domain}, up {up}: {pixel:?}, expected channel {channel} {expected}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn bounded_medium_context_preserves_local_and_world_coordinates() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(10.);
    value["render"]["background"]=json!([0,0,0]);
    value["nodes"][0]["translation"]=json!([1,0,0]);
    value["nodes"][1].as_object_mut().unwrap().remove("medium");
    value["programs"]=json!({"coordinates":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m;m.absorption=float3(.2);m.emission=float3(2.+c.position.x,2.+c.world_position.x,0.);return m;}"}});
    value["materials"]["fog"]=json!({"program":"coordinates"});
    value["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh","translation":[1,0,-2],"geometry":{"std":"box","size":[10,10,1]},"materials":["fog"]}));
    let expected=-(-0.2_f32).exp_m1()/0.2;
    for pixel in render(&r,value)? {assert!((pixel[1]-pixel[0]-expected).abs()<0.002,"{pixel:?}, expected channel difference {expected}");}
    Ok(())
}

#[test]
fn layered_source_reconstruction_matches_view_depth_reference() -> Result<()> {
    let Some(mut r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(2.1);
    value["render"]["resolution"]=json!([32,16]);
    r.reference_medium_grid=Some((32,3,0.));
    value["nodes"][0]=json!({"type":"camera","name":"camera","material":{"program":"std/camera","inputs":{"lens":8,"clip_start":0.01,"clip_end":2.1}}});
    value["programs"]=json!({"layers":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m;bool front=c.position.z > -(0.01+2.09/3.);m.absorption=float3(front ? 1. : 4.);m.emission=float3(front ? 1. : 10.);return m;}"}});
    value["materials"]["fog"]=json!({"program":"layers"});
    cover(&mut value);
    value["buffers"]["p"]["data"]=json!([[-1000,-1000,-2],[1000,-1000,-2],[0,1000,-2]]);
    // Compare the cached three-cell representation against direct integration
    // on each actual ray (tile one). Both use identical exact depth partitions.
    let actual=render(&r,value.clone())?;
    r.reference_medium_grid=Some((1,3,0.));
    let reference=render(&r,value)?;
    for (a,b) in actual.iter().zip(&reference) {for c in 0..3 {assert!((a[c]-b[c]).abs()<0.002,"cached {a:?}, receiver-ray reference {b:?}");}}
    Ok(())
}

/// A glass ray missing all surfaces still crosses the infinite world medium.
#[test]
fn ray_glass_escape_integrates_world_fog() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(50.);
    value["render"]["settings"]["refraction"]=json!("ray");
    value["render"]["background"]=json!("environment");
    value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.1);
    value["nodes"][1]["energy"]=json!(1.);
    value["nodes"][1]["color"]=json!([1,1,1]);
    value["materials"]["glass"]=json!({"inputs":{"specular_weight":0,"specular_ior":1,
        "specular_roughness":0,"transmission_weight":1}});
    value["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh","translation":[0,0,-2],
        "geometry":{"std":"box","size":[1,1,1]},"materials":["glass"]}));
    let expected=[0.3,0.2,0.1]; // emission / extinction, independent of distance.
    for pixel in render(&r,value)? {
        for channel in 0..3 {assert!((pixel[channel]-expected[channel]).abs()<0.001,
            "glass escape {pixel:?}, infinite fog {expected:?}");}
    }
    Ok(())
}

/// A null dielectric cannot change the transport of a bounded medium, including
/// the final segment from the last glass boundary to the medium's real exit.
#[test]
fn ray_glass_preserves_bounded_medium_transport() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut value=scene(50.);
    value["render"]["settings"]["refraction"]=json!("ray");
    value["render"]["background"]=json!("environment");
    value["nodes"][0]["material"]["inputs"]["ortho_scale"]=json!(0.1);
    value["nodes"][1].as_object_mut().unwrap().remove("medium");
    value["nodes"][1]["energy"]=json!(1.);
    value["nodes"][1]["color"]=json!([1,1,1]);
    value["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh","translation":[0,0,-2],
        "geometry":{"std":"box","size":[2,2,6]},"materials":["fog"]}));
    let direct=render(&r,value.clone())?;
    value["materials"]["glass"]=json!({"inputs":{"specular_weight":0,"specular_ior":1,
        "specular_roughness":0,"transmission_weight":1}});
    value["nodes"].as_array_mut().unwrap().push(json!({"type":"mesh","translation":[0,0,-2],
        "geometry":{"std":"box","size":[1,1,1]},"materials":["glass"]}));
    for (glass,direct) in render(&r,value)?.iter().zip(direct) {
        for channel in 0..3 {assert!((glass[channel]-direct[channel]).abs()<0.001,
            "bounded medium through identity glass {glass:?}, direct {direct:?}");}
    }
    Ok(())
}
