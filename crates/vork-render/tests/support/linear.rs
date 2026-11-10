//! Fixed-sample scene-linear comparison, independent of display quantization.
const RMSE_LIMIT: f64 = 1e-5;
const MAX_ABS_LIMIT: f64 = 1e-4;
fn linear_errors<const N: usize>(a: &[[f32; N]], b: &[[f32; N]]) -> [(f64, f64); N] {
    assert_eq!(a.len(), b.len(), "linear image dimensions");
    assert!(!a.is_empty(), "linear image readback was not enabled");
    std::array::from_fn(|channel| {
        let mut squared = 0_f64;
        let mut maximum = 0_f64;
        for (a, b) in a.iter().zip(b) {
            assert!(a[channel].is_finite() && b[channel].is_finite(), "non-finite linear output");
            let error = (f64::from(a[channel]) - f64::from(b[channel])).abs();
            squared += error * error;
            maximum = maximum.max(error);
        }
        ((squared / a.len() as f64).sqrt(), maximum)
    })
}
#[allow(dead_code)]
pub fn assert_linear_eq<const N: usize>(a: &[[f32; N]], b: &[[f32; N]]) {
    for (channel, (rmse, maximum)) in linear_errors(a,b).into_iter().enumerate() {
        assert!(rmse <= RMSE_LIMIT && maximum <= MAX_ABS_LIMIT,
            "linear channel {channel}: RMSE {rmse} (limit 1e-5), max-abs {maximum} (limit 1e-4)");
    }
}

/// A test render whose equality compares scene-linear samples with the tolerances
/// above. Dereferencing exposes display bytes for existing encoding/colour checks.
#[derive(Clone)]
#[allow(dead_code)]
pub struct LinearImage {
    pub rgba: Vec<u8>,
    pub linear: Vec<[f32; 4]>,
}
impl std::ops::Deref for LinearImage {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target { &self.rgba }
}
impl PartialEq for LinearImage {
    fn eq(&self, other: &Self) -> bool {
        let mut equivalent=true;
        for (channel,(rmse,maximum)) in linear_errors(&self.linear,&other.linear).into_iter().enumerate() {
            if rmse>RMSE_LIMIT || maximum>MAX_ABS_LIMIT {
                eprintln!("linear channel {channel}: RMSE {rmse}, max-abs {maximum}");
                equivalent=false;
            }
        }
        equivalent
    }
}
impl<'a> IntoIterator for &'a LinearImage {
    type Item = &'a u8;
    type IntoIter = std::slice::Iter<'a,u8>;
    fn into_iter(self) -> Self::IntoIter {self.rgba.iter()}
}
impl IntoIterator for LinearImage {
    type Item = u8;
    type IntoIter = std::vec::IntoIter<u8>;
    fn into_iter(self) -> Self::IntoIter {self.rgba.into_iter()}
}
impl std::fmt::Debug for LinearImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinearImage").field("pixels",&self.linear.len())
            .field("encoded_bytes",&self.rgba.len()).finish()
    }
}

/// View-axis depth in metres, before integer export quantization.
#[allow(dead_code)]
pub fn assert_depth_eq(a: &[f32], b: &[f32]) {
    let a: Vec<_> = a.iter().map(|&v| [v]).collect();
    let b: Vec<_> = b.iter().map(|&v| [v]).collect();
    assert_linear_eq(&a, &b);
}
