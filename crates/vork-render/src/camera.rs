//! OpenCV camera output; renderer matrices never enter the file format.
use glam::{Mat4, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Orthographic {
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct CameraRecord {
    #[serde(rename = "K", skip_serializing_if = "Option::is_none")]
    pub k: Option<[[f32; 3]; 3]>,
    #[serde(rename = "R")]
    pub r: [[f32; 3]; 3],
    pub t: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dist: Option<[f32; 5]>,
    pub image_size: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orthographic: Option<Orthographic>,
    #[serde(skip)]
    pub(crate) depth_unit_m: f32,
}
impl CameraRecord {
    pub(crate) fn new(
        view: Mat4,
        projection: Mat4,
        image_size: [u32; 2],
        height: Option<f32>,
        far: f32,
    ) -> Self {
        let [w, h] = image_size.map(|v| v as f32);
        // An orthographic shift is a translation of the view window. Encode it
        // in OpenCV extrinsics, whose orthographic record has no principal point.
        let mut view = view;
        if height.is_some() {
            view.w_axis.x += projection.w_axis.x / projection.x_axis.x;
            view.w_axis.y += projection.w_axis.y / projection.y_axis.y;
        }
        let cv = (Mat4::from_scale(Vec3::new(1., -1., -1.)) * view)
            .transpose()
            .to_cols_array_2d();
        Self {
            k: height.is_none().then_some([
                [
                    projection.x_axis.x * w * 0.5,
                    0.,
                    (w - 1.) * 0.5 - projection.z_axis.x * w * 0.5,
                ],
                [
                    0.,
                    projection.y_axis.y * h * 0.5,
                    (h - 1.) * 0.5 + projection.z_axis.y * h * 0.5,
                ],
                [0., 0., 1.],
            ]),
            r: std::array::from_fn(|row| std::array::from_fn(|col| cv[row][col])),
            t: std::array::from_fn(|row| cv[row][3]),
            dist: Some([0.; 5]),
            image_size,
            orthographic: height.map(|height| Orthographic { height }),
            depth_unit_m: far / 65535.,
        }
    }
    pub(crate) fn into_value(self) -> serde_json::Value {
        serde_json::to_value(self).expect("camera numbers are finite")
    }
}

/// Constant fields live at clip scope; a frame contains only its overrides.
/// `frame_start` is the absolute index of `frames[0]` for frame sequences; stills omit it.
pub(crate) fn clip(frames: &[CameraRecord], fps: vork_scene::Time, frame_start: Option<usize>) -> anyhow::Result<serde_json::Value> {
    let mut values = vec![serde_json::Map::new(); frames.len()];
    let mut common = serde_json::Map::new();
    fn field<T: serde::Serialize + PartialEq>(frames: &[CameraRecord], values: &mut [serde_json::Map<String, serde_json::Value>], common: &mut serde_json::Map<String, serde_json::Value>, name: &str, get: impl for<'a> Fn(&'a CameraRecord) -> Option<&'a T>) -> anyhow::Result<()> {
        let first = frames.first().and_then(&get);
        if frames.iter().all(|frame| get(frame) == first) {
            if let Some(value) = first {common.insert(name.into(), serde_json::to_value(value)?);}
        } else {
            for (frame, output) in frames.iter().zip(values) {
                if let Some(value) = get(frame) {output.insert(name.into(), serde_json::to_value(value)?);}
            }
        }
        Ok(())
    }
    // Compare typed fields before serialization: constant camera matrices need
    // only one JSON conversion, rather than one conversion per output frame.
    field(frames, &mut values, &mut common, "K", |r| r.k.as_ref())?;
    field(frames, &mut values, &mut common, "R", |r| Some(&r.r))?;
    field(frames, &mut values, &mut common, "t", |r| Some(&r.t))?;
    field(frames, &mut values, &mut common, "dist", |r| r.dist.as_ref())?;
    field(frames, &mut values, &mut common, "image_size", |r| Some(&r.image_size))?;
    field(frames, &mut values, &mut common, "orthographic", |r| r.orthographic.as_ref())?;
    common.insert("version".into(), 2.into());
    common.insert("fps".into(), fps.json());
    if let Some(start) = frame_start {
        common.insert("frame_start".into(), start.into());
    }
    common.insert("frames".into(), values.into_iter().map(serde_json::Value::Object).collect::<Vec<_>>().into());
    Ok(common.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shifted_principal_point_and_orthographic_window_match_projection() {
        let mut shift = Mat4::IDENTITY;
        shift.w_axis.x = -0.4;
        shift.w_axis.y = 0.2;
        let projection = shift * Mat4::perspective_rh(0.8, 2., 0.1, 100.);
        let camera = CameraRecord::new(Mat4::IDENTITY, projection, [200, 100], None, 100.);
        assert!((camera.k.unwrap()[0][2] - 59.5).abs() < 1e-5);
        assert!((camera.k.unwrap()[1][2] - 39.5).abs() < 1e-5);
        let projection = shift * Mat4::orthographic_rh(-4., 4., -2., 2., 0.1, 100.);
        let camera = CameraRecord::new(Mat4::IDENTITY, projection, [200, 100], Some(4.), 100.);
        assert!((camera.t[0] + 1.6).abs() < 1e-5);
        assert!((camera.t[1] + 0.4).abs() < 1e-5);
    }
    #[test]
    fn clip_factors_constants_and_keeps_projection_cuts() -> anyhow::Result<()> {
        let p = CameraRecord::new(Mat4::IDENTITY, Mat4::IDENTITY, [96, 64], None, 10.);
        let o = CameraRecord::new(Mat4::IDENTITY, Mat4::IDENTITY, [96, 64], Some(4.), 10.);
        let data = clip(&[p, o], vork_scene::Time::integer(24), None)?;
        assert_eq!(data["version"], 2);
        assert_eq!(data["fps"], 24.);
        assert_eq!(data["image_size"], serde_json::json!([96, 64]));
        assert_eq!(data["t"], serde_json::json!([0., 0., 0.]));
        assert!(data.get("K").is_none());
        assert!(data["frames"][0].get("K").is_some());
        assert!(data["frames"][1].get("K").is_none());
        assert_eq!(data["frames"][1]["orthographic"]["height"], 4.);
        assert!(data["frames"][0].get("image_size").is_none());
        assert!(clip(&[p, p], vork_scene::Time::integer(24), None)?["frames"][0].as_object().unwrap().is_empty());
        Ok(())
    }
}
