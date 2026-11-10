use crate::{FrameOutput, FrameReadback, ReadbackOutputs};
use anyhow::{Result, anyhow};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

use crate::constants::CONTROL_PNG_COMPRESSION;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Aov(String),
    Color,
    Depth,
    ObjectIndex,
    ViewNormal,
    Motion,
    Camera,
}
impl std::str::FromStr for Output {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        if let Some(name)=value.strip_prefix("aov:") {
            // AOV layers share the EXR namespace with built-in channels.
            return if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) {Err("AOV output requires a name using ASCII letters, digits, _ or -".into())}
            else if vork_scene::v2::OUTPUT_CHANNELS.contains(&name) {Err(format!("AOV {name:?} collides with a built-in channel and EXR layer; rename the AOV"))}
            else {Ok(Self::Aov(name.to_owned()))};
        }
        Self::builtin(value).ok_or_else(|| {
            ::vork_scene::v2::diagnostics::unknown("output channel", value, vork_scene::v2::OUTPUT_CHANNELS.iter().copied())
        })
    }
}
impl Output {
    /// A built-in channel by its token; vork_scene::v2::OUTPUT_CHANNELS lists the tokens.
    fn builtin(name: &str) -> Option<Self> {
        Some(match name {
            "color" => Self::Color,
            "depth" => Self::Depth,
            "object_index" => Self::ObjectIndex,
            "view_normal" => Self::ViewNormal,
            "motion" => Self::Motion,
            "camera" => Self::Camera,
            "shadow_catcher" => Self::Aov("shadow_catcher".into()),
            _ => return None,
        })
    }
    /// Every built-in channel, the default selection before declared AOVs.
    pub fn builtins() -> impl Iterator<Item = Self> {
        vork_scene::v2::OUTPUT_CHANNELS.iter().map(|name| Self::builtin(name).expect("every listed channel token parses"))
    }
}
#[derive(Clone)]
pub struct Outputs(pub Vec<Output>);
impl Outputs {
    pub fn new(outputs: Vec<Output>) -> Self {
        let mut unique = Vec::new();
        for output in outputs {
            if !unique.contains(&output) {
                unique.push(output);
            }
        }
        Self(unique)
    }
    pub fn has(&self, output: Output) -> bool {
        self.0.contains(&output)
    }
    pub fn readback(&self, packed: bool, float: bool) -> ReadbackOutputs {
        ReadbackOutputs {
            aovs: self.0.iter().any(|o|matches!(o,Output::Aov(_))),
            color: packed && self.has(Output::Color),
            depth_ids: (packed && self.has(Output::Depth)) || self.has(Output::ObjectIndex),
            normals: packed && self.has(Output::ViewNormal),
            motion: packed && self.has(Output::Motion),
            float: crate::FloatOutputs {
                color: float && self.has(Output::Color),
                depth: float && self.has(Output::Depth),
                normals: float && self.has(Output::ViewNormal),
                motion: float && self.has(Output::Motion),
            },
        }
    }
}
enum Input {
    Frame {
        frame: usize,
        pixels: FrameReadback,
    },
    Unpacked {
        frame: usize,
        pixels: Result<Arc<FrameOutput>>,
        lease: Option<Arc<crate::ring::Lease>>,
    },
    /// A refused frame: its output orders complete with no files.
    Skip {
        frame: usize,
    },
    Finish,
}
enum Job {
    Unpack {
        frame: usize,
        pixels: FrameReadback,
    },
    Plane {
        order: usize,
        frame: usize,
        outputs: Outputs,
        exr: bool,
        pixels: Arc<FrameOutput>,
        lease: Option<Arc<crate::ring::Lease>>,
    },
}
type Files = Vec<(String, Vec<u8>)>;
fn encode(w: u32, h: u32, p: &FrameOutput, outputs: &Outputs) -> Result<Files> {
    use png::{
        BitDepth::{Eight, Sixteen},
        ColorType::{Grayscale, Rgb, Rgba},
    };
    let mut files = Vec::new();
    let mut png = |suffix: &str, color, depth, data: &[u8]| -> Result<()> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, w, h);
            encoder.set_color(color);
            encoder.set_depth(depth);
            encoder.set_compression(CONTROL_PNG_COMPRESSION);
            encoder.write_header()?.write_image_data(data)?;
        }
        files.push((suffix.to_owned(), bytes));
        Ok(())
    };
    if outputs.has(Output::Color) {
        png(".png", Rgba, Eight, &p.rgba)?;
    }
    // Keep one output item per pixel. Flattening byte arrays through nested
    // iterators left a scalar Iterator::next state machine on this hot path;
    // collecting fixed-size records lets LLVM vectorize the endian conversion.
    // PNG stores 16-bit components big-endian on every host.
    if outputs.has(Output::Depth) {
        let words: Vec<_> = p.depth_u16.iter().map(|v| v.to_be()).collect();
        png(".depth.png", Grayscale, Sixteen, bytemuck::cast_slice(&words))?;
    }
    if outputs.has(Output::ObjectIndex) {
        let words: Vec<_> = p.ids.iter().map(|v| v.to_be()).collect();
        png(".object_index.png", Grayscale, Sixteen, bytemuck::cast_slice(&words))?;
    }
    if outputs.has(Output::ViewNormal) {
        png(".view_normal.png", Rgb, Eight, &p.normals)?;
    }
    if outputs.has(Output::Motion) {
        let words: Vec<_> = p.motion.iter().map(|v| [v[0].to_be(), v[1].to_be(), 0u16]).collect();
        png(".motion.png", Rgb, Sixteen, bytemuck::cast_slice(&words))?;
    }
    for output in &outputs.0 {
        if let Output::Aov(name)=output {
            let aov=p.aovs.get(name).ok_or_else(||anyhow!("missing AOV {name}"))?;
            let components=if aov.kind==vork_scene::v2::AovType::Value {1} else {3};
            let bytes:Vec<_>=aov.pixels.iter().flat_map(|v|v[..components].iter().flat_map(|c|((c.clamp(0.,1.)*65535.).round() as u16).to_be_bytes())).collect();
            png(&if name=="shadow_catcher" {".shadow_catcher.png".into()} else {format!(".aov.{name}.png")},if components==1 {Grayscale} else {Rgb},Sixteen,&bytes)?;
        }
    }
    Ok(files)
}
fn float_channels<'a, const N: usize>(
    names: [&'a str; N],
    values: &Option<Vec<[f32; N]>>,
) -> Result<Vec<(&'a str, Vec<f32>)>> {
    let values = values
        .as_ref()
        .ok_or_else(|| anyhow!("EXR channels require float readback"))?;
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(c, name)| (name, values.iter().map(|v| v[c]).collect()))
        .collect())
}

/// OpenEXR stores linear HDR and unquantized control signals; RGBA is premultiplied.
#[cfg(test)]
fn encode_exr(w: u32, h: u32, p: &FrameOutput, outputs: &Outputs, codec: crate::ExrCodec) -> Result<Files> {
    encode_exr_clock(w,h,p,outputs,codec,None)
}
fn encode_exr_clock(w: u32, h: u32, p: &FrameOutput, outputs: &Outputs, codec: crate::ExrCodec, fps: Option<(i32,u32)>) -> Result<Files> {
    use exr::prelude::*;
    let mut all_channels = Vec::new();
    let count = (w * h) as usize;
    for output in &outputs.0 {
        // Plain OpenEXR: beauty R,G,B,A and depth Z in the default layer; every other
        // channel is <name>.<channel>.
        let (name, channels): (&str, Vec<(&str, Vec<f32>)>) = match output {
            Output::Color => (
                "",
                float_channels(["R", "G", "B", "A"], &p.linear_rgba)?,
            ),
            Output::Depth => (
                "",
                vec![(
                    "Z",
                    p.depth_f32
                        .as_ref()
                        .ok_or_else(|| anyhow!("EXR depth requires float readback"))?
                        .clone(),
                )],
            ),
            Output::ObjectIndex => ("object_index", vec![("X", p.ids.iter().map(|v| *v as f32).collect())]),
            Output::ViewNormal => ("view_normal", float_channels(["X", "Y", "Z"], &p.normals_f32)?),
            Output::Motion => ("motion", float_channels(["X", "Y"], &p.motion_f32)?),
            Output::Camera => continue,
            Output::Aov(name) => {
                let aov=p.aovs.get(name).ok_or_else(||anyhow!("missing AOV {name}"))?;
                let labels:&[&str]=if aov.kind==vork_scene::v2::AovType::Value {&["X"]} else {&["R","G","B"]};
                (name.as_str(),labels.iter().enumerate().map(|(i,label)|(*label,aov.pixels.iter().map(|v|v[i]).collect())).collect())
            },
        };
        anyhow::ensure!(
            channels.iter().all(|(_, v)| v.len() == count),
            "EXR channel size mismatch"
        );
        all_channels.extend(channels.into_iter().map(|(channel, values)| {
            AnyChannel::new(
                exr::meta::attribute::Text::from_slice_unchecked(if name.is_empty() { channel.to_owned() } else { format!("{name}.{channel}") }.as_bytes()),
                FlatSamples::F32(values),
            )
        }));
    }
    let mut files = Vec::new();
    if !all_channels.is_empty() {
        let image = Image::from_layer(Layer::new(
            (w as usize, h as usize),
            LayerAttributes {frames_per_second:fps,..Default::default()},
            codec.encoding(),
            AnyChannels::sort(all_channels.into()),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write().non_parallel().to_buffered(&mut bytes)?;
        files.push((".exr".into(), bytes.into_inner()));
    }
    Ok(files)
}

/// One core-sized pool unpacks readbacks and compresses individual planes.
/// Ring leases bound mapped pixels, unpacked pixels, and ordered results.
pub struct FrameOutputWriter {
    jobs: mpsc::Sender<Input>,
    coordinator: thread::JoinHandle<Result<()>>,
    workers: Vec<thread::JoinHandle<()>>,
    writer: thread::JoinHandle<Result<()>>,
    pub worker_count: usize,
}
impl FrameOutputWriter {
    pub fn new(
        out: PathBuf,
        w: u32,
        h: u32,
        outputs: Outputs,
        exr: bool,
        exr_codec: crate::ExrCodec,
        pending_frames: usize,
        // Absolute index of the first pushed frame; output order counts from it.
        first_frame: usize,
        fps: (i32,u32),
    ) -> Self {
        let plane_count = if exr { 1 } else { outputs.0.len() };
        let worker_count = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(pending_frames.max(1) * plane_count.max(1));

        let (requests, input) = mpsc::channel::<Input>();
        let (jobs, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let (done, results) = mpsc::channel();
        let workers = (0..worker_count)
            .map(|_| {
                let rx = rx.clone();
                let done = done.clone();
                let requests = requests.clone();
                thread::spawn(move || {
                    loop {
                        let Ok(job) = rx.lock().unwrap().recv() else {
                            break;
                        };
                        match job {
                            Job::Unpack { frame, pixels } => {
                                let lease = pixels.credit.clone();
                                let pixels =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        pixels.unpack()
                                    }))
                                    .map(Arc::new)
                                    .map_err(|_| anyhow!("readback unpack worker panicked"));
                                if requests
                                    .send(Input::Unpacked {
                                        frame,
                                        pixels,
                                        lease,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Job::Plane {
                                order,
                                frame,
                                outputs,
                                exr,
                                pixels,
                                lease,
                            } => {
                                let files =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        if exr {
                                            encode_exr_clock(w, h, &pixels, &outputs, exr_codec,Some(fps))
                                        } else {
                                            encode(w, h, &pixels, &outputs)
                                        }
                                    }))
                                    .unwrap_or_else(|_| {
                                        Err(anyhow!("frame compression worker panicked"))
                                    });
                                if done.send((order, frame, files, lease)).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                })
            })
            .collect();
        // Only this coordinator produces pool jobs. Explicit Finish lets it
        // close the pool after all unpack jobs have expanded into plane jobs.
        let coordinator = thread::spawn(move || {
            let mut unpacking = 0;
            let mut finished = false;
            for request in input {
                match request {
                    Input::Frame { frame, pixels } => {
                        unpacking += 1;
                        jobs.send(Job::Unpack { frame, pixels })?;
                    }
                    Input::Unpacked {
                        frame,
                        pixels,
                        lease,
                    } => {
                        unpacking -= 1;
                        let groups = if exr {
                            vec![outputs.clone()]
                        } else {
                            outputs.0.iter().map(|o| Outputs(vec![o.clone()])).collect()
                        };
                        for (i, outputs) in groups.into_iter().enumerate() {
                            let order = (frame - first_frame) * plane_count + i;
                            match &pixels {
                                Ok(pixels) => jobs.send(Job::Plane {
                                    order,
                                    frame,
                                    outputs,
                                    exr,
                                    pixels: pixels.clone(),
                                    lease: lease.clone(),
                                })?,
                                Err(error) => done.send((
                                    order,
                                    frame,
                                    Err(anyhow!(error.to_string())),
                                    lease.clone(),
                                ))?,
                            }
                        }
                    }
                    Input::Skip { frame } => {
                        for i in 0..plane_count {
                            done.send(((frame - first_frame) * plane_count + i, frame, Ok(Files::new()), None))?;
                        }
                    }
                    Input::Finish => finished = true,
                }
                if finished && unpacking == 0 {
                    break;
                }
            }
            Ok(())
        });
        let writer = thread::spawn(move || {
            let mut pending = BTreeMap::new();
            let mut next = 0;
            let mut error = None;
            for (order, frame, files, lease) in results {
                pending.insert(order, (frame, files, lease));
                while let Some((frame, files, _lease)) = pending.remove(&next) {
                    let written = files.and_then(|files| {
                        for (suffix, bytes) in files {
                            let path = if exr {
                                let stem = out.file_stem().unwrap().to_string_lossy();
                                let stem = stem.replace("%06d", &format!("{frame:06}"));
                                out.with_file_name(format!("{stem}{suffix}"))
                            } else if !crate::shot::directory_marked(&out) && out.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
                                // A one-still PNG destination: colour at the path, other channels beside it.
                                if suffix == ".png" {
                                    out.clone()
                                } else {
                                    let stem = out.file_stem().unwrap().to_string_lossy();
                                    out.with_file_name(format!("{stem}{suffix}"))
                                }
                            } else {
                                out.join(format!("{frame:06}{suffix}"))
                            };
                            std::fs::write(path, bytes)?;
                        }
                        Ok(())
                    });
                    if let Err(e) = written {
                        if error.is_none() {
                            error = Some(e);
                        }
                    }
                    next += 1;
                }
            }
            if !pending.is_empty() {
                return Err(anyhow!("frame output worker left a gap in output order"));
            }
            error.map_or(Ok(()), Err)
        });
        Self {
            jobs: requests,
            coordinator,
            workers,
            writer,
            worker_count,
        }
    }
    pub fn push(&self, frame: usize, pixels: FrameReadback) -> Result<()> {
        self.jobs
            .send(Input::Frame { frame, pixels })
            .map_err(|_| anyhow!("frame output workers stopped"))
    }
    /// Write nothing for a refused frame, keeping later frames in order.
    pub fn skip(&self, frame: usize) -> Result<()> {
        self.jobs
            .send(Input::Skip { frame })
            .map_err(|_| anyhow!("frame output workers stopped"))
    }
    pub fn finish(self) -> Result<()> {
        let _ = self.jobs.send(Input::Finish);
        drop(self.jobs);
        let coordinated = self
            .coordinator
            .join()
            .map_err(|_| anyhow!("frame output coordinator panicked"));
        let mut panicked = false;
        for worker in self.workers {
            panicked |= worker.join().is_err();
        }
        let result = self
            .writer
            .join()
            .map_err(|_| anyhow!("frame output writer panicked"))?;
        coordinated??;
        if panicked {
            return Err(anyhow!("frame output worker panicked"));
        }
        result
    }
}

#[cfg(test)]
mod exr_tests {
    use super::*;
    #[test]
    fn aov_channels_cannot_shadow_built_in_layers() {
        assert!("aov:heat".parse::<Output>().is_ok());
        assert!("aov:color".parse::<Output>().unwrap_err().contains("collides"));
        assert!("aov:object_index".parse::<Output>().unwrap_err().contains("collides"));
        assert_eq!(Output::builtins().count(), vork_scene::v2::OUTPUT_CHANNELS.len());
        assert!("ZIP".parse::<crate::ExrCodec>().is_err());
    }
    #[test]
    fn channel_grammar_is_namespaced_and_builtin_defaults_are_stable() {
        // Unimplemented selections are unknown channels, not reserved promises.
        for token in ["aov:", "aov:a:b", "aov:a.b", "aov:../file", "unknown:x", "cryptomatte_object"] {
            assert!(token.parse::<Output>().is_err(), "{token}");
        }
        for token in ["cryptomatte:object", "cryptomatte:material", "cryptomatte:asset", "lightgroup:key"] {
            assert!(token.parse::<Output>().unwrap_err().starts_with("unknown output channel"), "{token}");
        }
        let defaults: Vec<Output> = Output::builtins().collect();
        assert_eq!(defaults, vec![Output::Color, Output::Depth, Output::ObjectIndex, Output::ViewNormal, Output::Motion, Output::Camera, Output::Aov("shadow_catcher".into())]);
    }
    #[test]
    fn multilayer_and_png_control_units_round_trip() -> Result<()> {
        use exr::prelude::*;
        let mut pixels = FrameOutput {
            aovs: [("heat".into(), crate::frame::AovPixels {
                kind: vork_scene::v2::AovType::Value,
                pixels: vec![[0.25, 0., 0., 0.], [0.75, 0., 0., 0.]],
            })].into(),
            rgba: vec![128, 128, 255, 128, 0, 0, 0, 0],
            linear_rgba: Some(vec![[0.09, 0.2158605 * 0.5, 2., 0.5], [0., 0., 0., 0.]]),
            depth_f32: Some(vec![1.2345679, 1e10]),
            normals_f32: Some(vec![[0.1234567, 0.2345678, 0.964], [0., 0., 0.]]),
            motion_f32: Some(vec![[0., 0.], [0., 0.]]),
            depth_u16: vec![100, 0],
            ids: vec![65535, 0],
            normals: vec![255, 128, 128, 0, 0, 0],
            motion: vec![[32768, 32768], [32768, 32768]],
            camera: serde_json::json!({}),
            exposure: None,
            timing: crate::FrameTiming {
                cpu_prepare_ms: 0.,
                cpu_submit_ms: 0.,
                cpu_finish_ms: 0.,
                denoise_wait_ms: 0.,
                denoise_pack_ms: 0.,
                denoise_filter_ms: 0.,
                denoise_upload_ms: 0.,
                gpu_ms: 0.,
                nonfinite: None,
                dropped_light_samples: 0,
                gpu_elapsed_ms: 0.,
                timing_source: "unmeasured",
                gpu_spans_ms: vec![],
                sample_count: 1,
            },
        };
        // The public PNG protocol uses network byte order, including the zero
        // blue component of the RGB16 motion image. Decode the actual files so
        // this also checks their PNG type, depth and dimensions.
        let pngs = encode(1, 2, &pixels, &Outputs(vec![Output::Depth, Output::ObjectIndex, Output::Motion]))?;
        for (suffix, color, expected) in [
            (".depth.png", png::ColorType::Grayscale, vec![0, 100, 0, 0]),
            (".object_index.png", png::ColorType::Grayscale, vec![255, 255, 0, 0]),
            (".motion.png", png::ColorType::Rgb, vec![128, 0, 128, 0, 0, 0, 128, 0, 128, 0, 0, 0]),
        ] {
            let bytes = &pngs.iter().find(|(name, _)| name == suffix).unwrap().1;
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
            let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut decoded)?;
            assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (1, 2, color, png::BitDepth::Sixteen));
            assert_eq!(&decoded[..info.buffer_size()], &expected, "{suffix}");
        }
        let files = encode_exr(
            1,
            2,
            &pixels,
            &Outputs(vec![
                Output::Color,
                Output::Depth,
                Output::ObjectIndex,
                Output::ViewNormal,
                Output::Motion,
                Output::Camera,
                Output::Aov("heat".into()),
            ]),
            crate::ExrCodec::default(),
        )?;
        assert_eq!(files.len(), 1);
        let image = read()
            .no_deep_data()
            .largest_resolution_level()
            .all_channels()
            .all_layers()
            .all_attributes()
            .from_buffered(std::io::Cursor::new(&files[0].1))?;
        assert_eq!(image.layer_data[0].encoding.compression, Compression::ZIP16);
        assert_eq!(image.layer_data.len(), 1);
        assert_eq!(image.layer_data[0].channel_data.list.len(), 12);
        let sample = |channel: &str| {
            image.layer_data[0]
                .channel_data
                .list
                .iter()
                .find(|c| c.name.to_string() == channel)
                .unwrap()
                .sample_data
                .values_as_f32()
                .collect::<Vec<_>>()
        };
        assert_eq!(sample("Z"), [1.2345679, 1e10]);
        assert_eq!(sample("object_index.X"), [65535., 0.]);
        assert_eq!(sample("heat.X"), [0.25, 0.75]);
        assert_eq!(sample("R"), [0.09, 0.]);
        assert_eq!(sample("G"), [0.2158605 * 0.5, 0.]);
        assert_eq!(sample("B"), [2., 0.]);
        assert_eq!(sample("A"), [0.5, 0.]);
        assert_eq!(sample("view_normal.X"), [0.1234567, 0.]);
        assert_eq!(sample("motion.X"), [0., 0.]);
        assert_eq!(sample("motion.Y"), [0., 0.]);
        // Beauty and depth sit in the default layer; every other layer is a
        // built-in channel name, reserved against AOV names at scene validation.
        for channel in &image.layer_data[0].channel_data.list {
            let name = channel.name.to_string();
            match name.split_once('.') {
                None => assert!(["R", "G", "B", "A", "Z"].contains(&name.as_str()), "{name}"),
                Some((layer, _)) => assert!(layer == "heat" || vork_scene::v2::OUTPUT_CHANNELS.contains(&layer), "{name}"),
            }
        }
        for name in ["none", "rle", "zips", "zip", "piz", "pxr24", "b44", "b44a", "dwaa", "dwab"] {
            let codec: crate::ExrCodec = name.parse().unwrap();
            let files = encode_exr(1, 2, &pixels, &Outputs(vec![Output::Color, Output::Depth, Output::ObjectIndex]), codec)?;
            let image = read().no_deep_data().largest_resolution_level().all_channels()
                .all_layers().all_attributes().from_buffered(std::io::Cursor::new(&files[0].1))?;
            assert_eq!(std::mem::discriminant(&image.layer_data[0].encoding.compression),
                std::mem::discriminant(&codec.encoding().compression), "{name}");
            let ids = image.layer_data[0].channel_data.list.iter()
                .find(|c| c.name.to_string() == "object_index.X").unwrap()
                .sample_data.values_as_f32().collect::<Vec<_>>();
            assert_eq!(ids, [65535., 0.], "{name} must retain exact object IDs");
        }
        // Declared lists beyond the former shader budget keep their names and
        // values through EXR and explicit --channels selections.
        for count in [0, 1, 3, 8, 32] {
            pixels.aovs = (0..count).map(|i| (format!("a{i}"), crate::frame::AovPixels {
                kind: if i % 2 == 0 {vork_scene::v2::AovType::Value} else {vork_scene::v2::AovType::Color},
                pixels: vec![[(i+1) as f32 / 64., 0.5, 0.75, 1.]; 2],
            })).collect();
            let outputs = Outputs::new(std::iter::once(Output::Color).chain((0..count).map(|i| Output::Aov(format!("a{i}")))).collect());
            let files = encode_exr(1, 2, &pixels, &outputs, crate::ExrCodec::default())?;
            let image = read().no_deep_data().largest_resolution_level().all_channels()
                .all_layers().all_attributes().from_buffered(std::io::Cursor::new(&files[0].1))?;
            let channels = &image.layer_data[0].channel_data.list;
            assert_eq!(channels.len(), 4 + (0..count).map(|i| if i%2==0 {1} else {3}).sum::<usize>());
            for i in 0..count {
                for (component, expected) in if i%2==0 {vec![("X", (i+1) as f32/64.)]} else {vec![("R", (i+1) as f32/64.), ("G", 0.5), ("B", 0.75)]} {
                    let name = format!("a{i}.{component}");
                    let channel = channels.iter().find(|c| c.name.to_string()==name).unwrap();
                    assert_eq!(channel.sample_data.values_as_f32().collect::<Vec<_>>(), vec![expected;2]);
                }
            }
            if count>0 {
                let selection: Output = format!("aov:a{}",count-1).parse().unwrap();
                let selected = Outputs::new(vec![selection]);
                let png = encode(1,2,&pixels,&selected)?;
                assert_eq!(png.len(),1);
                assert_eq!(png[0].0,format!(".aov.a{}.png",count-1));
                let files = encode_exr(1,2,&pixels,&selected,crate::ExrCodec::default())?;
                let image = read().no_deep_data().largest_resolution_level().all_channels()
                    .all_layers().all_attributes().from_buffered(std::io::Cursor::new(&files[0].1))?;
                assert!(image.layer_data[0].channel_data.list.iter().all(|c| c.name.to_string().starts_with(&format!("a{}.",count-1))));
            }
        }
        Ok(())
    }
}
