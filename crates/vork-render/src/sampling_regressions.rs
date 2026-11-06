//! Sampling contract tests kept separate from the shared sequence implementation.

use vork_slang::renderer_registry as registry;

#[test]
fn consumer_streams_do_not_overlap_light_dimensions() {
    use registry::*;
    let source = generate(true);
    let streams: Vec<_> = source.lines()
        .filter(|line| line.contains("const uint REG_SAMPLING_STREAM_"))
        .map(|line| {
            let (name, value) = line.split_once(" = ").unwrap();
            (name, value.trim_end_matches("u;").parse::<u32>().unwrap())
        }).collect();
    assert!(!streams.is_empty(), "must inspect registered consumers");
    for (i, (name, value)) in streams.iter().enumerate() {
        for (other, other_value) in &streams[..i] {
            assert_ne!(value, other_value, "{name} aliases {other}");
        }
    }
    // Any storage-buffer light array fits in the u32 byte-address domain.
    // Solve the modular stream equation instead of checking a few bench lights:
    // base + light*STEP + dimension == singleton (mod 2^32).
    let max_lights = u32::MAX / std::mem::size_of::<crate::lights::Light>() as u32;
    let mut inverse = REG_PIXEL_HASH_STREAM_STEP;
    for _ in 0..5 { inverse = inverse.wrapping_mul(2u32.wrapping_sub(REG_PIXEL_HASH_STREAM_STEP.wrapping_mul(inverse))); }
    assert_eq!(inverse.wrapping_mul(REG_PIXEL_HASH_STREAM_STEP), 1);
    for (name, value) in streams {
        if value == REG_SAMPLING_STREAM_SHADOW { continue; }
        for dimension in 0..3u32 {
            let light = value.wrapping_sub(REG_SAMPLING_STREAM_SHADOW)
                .wrapping_sub(dimension).wrapping_mul(inverse);
            assert!(light >= max_lights, "{name} aliases light {light}, dimension {dimension}");
        }
    }
    for difference in [1u32, 2u32] {
        let separation = difference.wrapping_mul(inverse);
        assert!(separation >= max_lights && separation.wrapping_neg() >= max_lights,
            "shadow dimensions overlap between addressable lights");
    }
}

#[test]
fn stratified_alpha_integrates_coverage_with_one_sample_error() {
    for stream in [registry::REG_SAMPLING_STREAM_RASTER_ALPHA, registry::REG_SAMPLING_STREAM_SHADOW_ALPHA] {
        for pixel in [[0, 0], [37, 91], [u32::MAX, 715]] {
            for count in [7, 64, 4096] {
                for opacity in [0., 0.001, 0.17, 0.4, 0.731, 0.999, 1.] {
                    let accepted = (0..count).filter(|&index|
                        crate::sampling::sample_2d(pixel, index, count, stream)[0] < opacity).count();
                    let error = (accepted as f32 / count as f32 - opacity).abs();
                    assert!(error <= 1. / count as f32 + f32::EPSILON,
                        "stream={stream}, count={count}, opacity={opacity}: {error}");
                }
            }
        }
    }
}

fn ggx_normal_moment_reference(x:f64) -> f64 {
    let b=1.-x;
    if b<0.125 {
        let mut sum=2./3.;let mut power=b;let mut k=1.;
        loop {
            let next=sum+2.*power/((2.*k+1.)*(2.*k+3.));
            if next==sum {return sum;}
            sum=next;power*=b;k+=1.;
        }
    }
    let t=b.sqrt();(1.-x*((1.+t)/x.sqrt()).ln()/t)/b
}

pub(crate) fn ggx_inverse_moment_reference(target:f64) -> f64 {
    let(mut lo,mut hi)=(1e-18,1.);
    for _ in 0..64 {
        let mid=(lo+hi)/2.;
        if ggx_normal_moment_reference(mid)>target {lo=mid;} else {hi=mid;}
    }
    (lo+hi)/2.
}

#[test]
fn ggx_normal_inverse_covers_the_float_domain() {
    use registry::*;
    for i in 0..=1000 {
        let z=-1.+2.*f64::from(i)/1000.;
        let deficit=(f64::from(REG_GGX_NORMAL_LOG_MIN)+(z+1.)*f64::from(REG_GGX_NORMAL_LOG_RANGE)/2.).exp();
        let expected=ggx_inverse_moment_reference(1.-deficit);
        let(mut b1,mut b2)=(0.,0.);
        for &c in REG_GGX_NORMAL_INVERSE[1..].iter().rev() {
            let next=2.*z*b1-b2+f64::from(c);b2=b1;b1=next;
        }
        let actual=deficit/(z*b1-b2+f64::from(REG_GGX_NORMAL_INVERSE[0]));
        assert!(((actual-expected)/expected).abs()<16.*f64::from(f32::EPSILON),"deficit {deficit}: alpha^2 {actual} vs {expected}");
    }
}
