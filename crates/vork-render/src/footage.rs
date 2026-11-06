//! Display-frame media selection, adapted from an earlier portable
//! decoder: timestamp/keyframe lookup, sequential raw-frame pipe, indexed backwards
//! seeks. No clip is decoded into an unbounded in-memory frame collection.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::{Path, PathBuf}, process::{Child, ChildStdout, Command, Stdio}};

pub fn is_media(path: &Path) -> bool {
    is_sequence(path) || path.extension().is_some_and(|e|
        ["mov", "mp4", "mkv"].iter().any(|v| e.eq_ignore_ascii_case(v)))
}

fn is_sequence(path:&Path)->bool {
    path.file_name().and_then(|n|n.to_str()).is_some_and(|name| {
        name.match_indices('%').any(|(at,_)| {
            let tail=&name[at+1..];
            tail.starts_with('d') || tail.strip_prefix('0').is_some_and(|digits| {
                let count=digits.bytes().take_while(u8::is_ascii_digit).count();
                count>0 && digits[count..].starts_with('d')
            })
        })
    })
}

pub enum Source {
    Sequence { files:Vec<PathBuf>, fps:Option<f32> },
    Video(Video),
}
impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        if is_sequence(path) {
            let files=sequence(path)?;
            let fps=if files[0].extension().is_some_and(|e|e.eq_ignore_ascii_case("exr")) {
                exr::meta::MetaData::read_from_file(&files[0],false)?.headers[0].own_attributes.frames_per_second
                    .map(|(n,d)|n as f32/d as f32)
            } else {None};
            ensure!(fps.is_none_or(|fps|fps.is_finite() && fps>0.),"EXR sequence framesPerSecond must be positive");
            return Ok(Self::Sequence {files,fps});
        }
        Ok(Self::Video(Video::open(path)?))
    }
    pub(crate) fn open_gpu(path:&Path,r:&crate::Renderer)->Result<Self> {
        let mut source=Self::open(path)?;
        if let Self::Video(video)=&mut source {
            // Use the installed NVIDIA adapter,
            // retaining the portable decoder for unsupported streams. FFmpeg
            // owns native decoder setup and format conversion; the existing
            // ordered texture-upload path remains the publication boundary.
            video.hardware=r.adapter_info.vendor==crate::constants::NVIDIA_VENDOR_ID;
        }
        Ok(source)
    }
    pub fn timeline(&self,fps:f32)->Result<Vec<f32>> {
        ensure!(fps.is_finite() && fps>0.,"footage fps must be positive");
        let mut times=match self {Self::Sequence {files,fps:source_fps}=>(0..files.len()).map(|k| {let fps=source_fps.unwrap_or(fps);let mut t=k as f32/fps;while (t as f64*fps as f64).floor()<(k as f64) {t=t.next_up();}t}).collect::<Vec<_>>(),Self::Video(video)=>video.times.iter().map(|t|*t as f32).collect()};
        times.dedup();Ok(times)
    }
    pub(crate) fn identity(&self)->Result<blake3::Hash> {
        let mut hash=blake3::Hasher::new();
        let files=match self {Self::Sequence {files,..}=>files.clone(),Self::Video(video)=>vec![video.path.clone()]};
        for file in files {let mut part=blake3::Hasher::new();part.update_reader(std::fs::File::open(file)?)?;hash.update(part.finalize().as_bytes());}
        Ok(hash.finalize())
    }
    pub(crate) fn frame_index(&self,time:f32,fps:f32)->usize {
        match self {
            Self::Sequence {files,fps:source_fps}=>((time.max(0.) as f64*source_fps.unwrap_or(fps) as f64).floor() as usize).min(files.len()-1),
            Self::Video(video)=>video.times.partition_point(|&t|(t as f32)<=time).saturating_sub(1),
        }
    }
    pub fn frame(&mut self, time: f32, fps: f32, max_side: u32) -> Result<image::DynamicImage> {
        ensure!(time.is_finite(), "image.time must be finite");
        match self {
            Self::Sequence {files:frames,fps:source_fps} => {
                let fps=source_fps.unwrap_or(fps);
                let index = ((time.max(0.) as f64 * fps as f64).floor() as usize).min(frames.len()-1);
                let path = &frames[index];
                crate::texture_source::decode(path, &std::fs::read(path)?, max_side)
                    .with_context(|| format!("decode sequence frame {}", path.display()))
            }
            Self::Video(video) => video.frame(time as f64, max_side),
        }
    }
}

fn sequence(path: &Path) -> Result<Vec<PathBuf>> {
    let name = path.file_name().context("sequence needs a filename")?.to_str().context("sequence filename must be UTF-8")?;
    let start = name.find('%').context("sequence pattern missing %")?;
    let tail = &name[start+1..];
    let end = tail.find('d').context("sequence pattern must use %d or %04d")?;
    let digits = &tail[..end];
    ensure!(digits.is_empty() || (digits.starts_with('0') && digits[1..].chars().all(|c| c.is_ascii_digit())), "sequence pattern must use %d or %04d");
    let width = if digits.is_empty() {0} else {digits.parse::<usize>()?};
    ensure!(width <= 20, "sequence field width exceeds 20 digits");
    let prefix = &name[..start];
    let suffix = &tail[end+1..];
    ensure!(!suffix.contains('%'), "sequence must have one frame field");
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut frames = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let file = entry.file_name();
        let Some(file) = file.to_str() else {continue};
        let Some(number) = file.strip_prefix(prefix).and_then(|n| n.strip_suffix(suffix)) else {continue};
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {continue;}
        let index = number.parse::<u64>()?;
        if format!("{index:0width$}") != number {continue;}
        frames.insert(index, entry.path());
    }
    ensure!(!frames.is_empty(), "sequence {} has no frames", path.display());
    let mut missing = Vec::new();
    let mut previous = *frames.first_key_value().unwrap().0;
    for &index in frames.keys().skip(1) {
        if index > previous+1 {
            missing.push(if index == previous+2 {format!("{}",previous+1)} else {format!("{}..{}",previous+1,index-1)});
        }
        previous = index;
    }
    ensure!(missing.is_empty(), "sequence {} missing frames: {}", path.display(), missing.join(", "));
    Ok(frames.into_values().collect())
}

struct Session { child: Child, stdout: ChildStdout, next: usize }
impl Drop for Session {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
pub struct Video {
    path: PathBuf,
    times: Vec<f64>,
    keyframes: Vec<(usize,f64)>,
    last_request: Option<usize>,
    #[cfg(test)]
    decoded: usize,
    size: [u32;2],
    session: Option<Session>,
    hardware: bool,
    frames: std::collections::VecDeque<(usize, image::DynamicImage)>,
}
impl Video {
    fn open(path: &Path) -> Result<Self> {
        // ffprobe and ffmpeg both apply container edit lists. Presentation times
        // are relative to the first displayed frame, never packet/decode order.
        let probe = Command::new("ffprobe").args(["-v","error","-select_streams","v:0","-show_frames","-show_streams","-show_entries",
            "frame=best_effort_timestamp,key_frame:stream=width,height,time_base:stream_side_data=rotation","-of","json"])
            .arg(path).output().context("video inputs require ffprobe and ffmpeg on PATH")?;
        ensure!(probe.status.success(), "probe video {}: {}", path.display(), String::from_utf8_lossy(&probe.stderr));
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let stream = &value["streams"][0];
        let mut size = [stream["width"].as_u64().context("video width missing")? as u32,
                        stream["height"].as_u64().context("video height missing")? as u32];
        if let Some(rotation) = stream["side_data_list"].as_array().and_then(|a|a.iter().find_map(|s|s["rotation"].as_i64())) {
            if rotation.rem_euclid(180) == 90 {size.swap(0,1);}
        }
        let (numerator,denominator)=stream["time_base"].as_str().context("video time base missing")?.split_once('/').context("invalid video time base")?;
        let tick=numerator.parse::<f64>()?/denominator.parse::<f64>()?;
        let mut times: Vec<f64> = value["frames"].as_array().context("video has no displayed frames")?.iter()
            .map(|f| f["best_effort_timestamp"].as_i64().map(|pts|pts as f64*tick).context("video frame has no presentation timestamp"))
            .collect::<Result<_>>()?;
        ensure!(!times.is_empty() && size.iter().all(|&n|n>0), "video has no displayed frames");
        // Only unique presentation times are seek anchors: a later keyframe
        // sharing a timestamp cannot identify which duplicate FFmpeg will emit.
        let keyframes=times.iter().enumerate().filter_map(|(i,&time)| {
            (i>0 && value["frames"][i]["key_frame"].as_u64()==Some(1)
                && times[i-1]<time && times.get(i+1).is_none_or(|&t|t>time))
                .then_some((i,time))
        }).collect();
        let first = times[0];
        for time in &mut times {*time -= first;}
        ensure!(times.iter().all(|t|t.is_finite()) && times.windows(2).all(|w|w[1]>=w[0]), "video presentation timestamps are invalid");
        Ok(Self {path:path.into(), times, keyframes, last_request:None, #[cfg(test)] decoded:0, size, session:None, hardware:false, frames:Default::default()})
    }
    fn frame(&mut self, time: f64, max_side:u32) -> Result<image::DynamicImage> {
        ensure!(self.size.iter().all(|&n|n<=max_side), "video {} exceeds GPU texture side {max_side}", self.path.display());
        let index = self.times.partition_point(|&t|(t as f32)<=time as f32).saturating_sub(1);
        let previous=self.last_request.replace(index);
        if let Some((_,frame)) = self.frames.iter().find(|(cached,_)|*cached==index) {return Ok(frame.clone());}
        // For a reverse walk retain the next predicted request too. The existing
        // bounded frame owner holds those two frames, not the whole decoded GOP.
        let preceding=previous.filter(|&p|p>index).map(|p|index.saturating_sub(p-index));
        let anchor=self.keyframes.iter().rev().find(|&&(i,_)|i<=index).copied();
        let start=anchor.map_or(0,|(i,_)|i);
        // A forward jump also seeks when its keyframe is beyond the live pipe.
        if self.session.as_ref().is_none_or(|s|s.next>index || s.next<start) {
            self.session = None;
            let mut command=Command::new("ffmpeg");
            command.args(["-v","error","-nostdin"]);
            if self.hardware {command.args(["-hwaccel","auto"]);}
            if let Some((_,time))=anchor {
                // Absolute container timestamps avoid assuming stream/container
                // start times agree. Round down at FFmpeg's microsecond boundary
                // so a fractional tick never discards the anchor itself.
                command.args(["-seek_timestamp","1","-ss",&format!("{:.6}",(time*1e6).floor()/1e6)]);
            }
            let mut child = command.arg("-i").arg(&self.path)
                .args(["-map","0:v:0","-an","-sn","-fps_mode","passthrough","-f","rawvideo","-pix_fmt","rgba64le","pipe:1"])
                .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().context("start video decoder")?;
            let stdout = child.stdout.take().unwrap();
            self.session = Some(Session {child,stdout,next:start});
        }
        let session = self.session.as_mut().unwrap();
        let mut bytes = vec![0; self.size[0] as usize * self.size[1] as usize * 8];
        let mut result=None;
        while session.next<=index {
            session.stdout.read_exact(&mut bytes).with_context(||format!("decode video {} frame {}",self.path.display(),session.next))?;
            #[cfg(test)] {self.decoded+=1;}
            if session.next==index || preceding==Some(session.next) {
                let pixels=bytes.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
                let image=image::DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(self.size[0],self.size[1],pixels).context("video frame size")?);
                if session.next==index {result=Some(image.clone());}
                self.frames.push_back((session.next,image));
                while self.frames.len()>crate::constants::FOOTAGE_DECODE_FRAMES {self.frames.pop_front();}
            }
            session.next+=1;
        }
        let image=result.context("video frame missing")?;
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore="requires a real GPU and pinned Slang"]
    fn sequence_compositor_samples_its_own_extent_and_updates_with_the_clock()->Result<()> {
        let Some(r)=crate::test_renderer()? else {return Ok(())};
        let root=std::env::temp_dir().join(format!("vork-footage-compositor-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let first=image::RgbaImage::from_fn(8,4,|x,_|image::Rgba(if x<4 {[255,0,0,255]} else {[0,255,0,255]}));
        first.save(root.join("1001.png"))?;
        image::RgbaImage::from_pixel(8,4,image::Rgba([0,0,255,255])).save(root.join("1002.png"))?;
        let input:crate::Scene=serde_json::from_value(serde_json::json!({"version":2,
            "render":{"resolution":[4,2],"fps":24,"motion_blur":{"shutter":1,"position":"end"},"compositor":["comp"],"settings":{"samples":4,"integrator":"path","denoiser":null}},
            "images":{"plate":{"file":"%04d.png"}},
            "programs":{"plate":{"source":"import vork; struct Inputs {Image plate;}; float4 composite(CompositeContext c,Inputs i) {return compositor_sample(i.plate,c.uv);}"}},
            "materials":{"comp":{"program":"plate","inputs":{"plate":{"image":"plate"}}}},
            "nodes":[{"type":"camera"}]}))?;
        let scene=r.install(input,&root)?;
        let mut slot=scene.create_frame_slot(&r,crate::ReadbackPrecision::Float)?;
        let outputs=crate::ReadbackOutputs {float:crate::FloatOutputs {color:true,..Default::default()},..Default::default()};
        slot.submit_outputs(&scene,&r,0.,outputs)?;
        let first=slot.collect(&r)?.linear_rgba.unwrap();
        assert_eq!(first[0],[1.,0.,0.,1.]);assert_eq!(first[3],[0.,1.,0.,1.]);
        slot.submit_outputs(&scene,&r,1./24.,outputs)?;
        assert!(slot.collect(&r)?.linear_rgba.unwrap().iter().all(|p|*p==[0.,0.,1.,1.]));
        // The same media clock also drives the World, without a compositor.
        let environment:crate::Scene=serde_json::from_value(serde_json::json!({"version":2,
            "render":{"resolution":[4,2],"fps":24,"settings":{"samples":1}},
            "images":{"plate":{"file":"%04d.png"}},
            "materials":{"environment_image":{"program":"std/environment_image","inputs":{"image":{"image":"plate"}}}},
            "nodes":[{"type":"camera"},{"type":"light","shape":"environment","material":"environment_image","energy":1}]}))?;
        let environment=r.install(environment,&root)?;
        let mut world=environment.create_frame_slot(&r,crate::ReadbackPrecision::Float)?;
        world.submit_outputs(&environment,&r,0.,outputs)?;
        let first=world.collect(&r)?.linear_rgba.unwrap();
        assert!(first.iter().any(|p|p[0]>0. || p[1]>0.));
        world.submit_outputs(&environment,&r,1./24.,outputs)?;
        for p in world.collect(&r)?.linear_rgba.unwrap() {assert!(p[0].abs()<0.001 && p[1].abs()<0.001 && (p[2]-1.).abs()<0.001,"retimed World: {p:?}");}
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn image_clock_uses_track_layers_for_reverse_hold_and_loop() -> Result<()> {
        let scene:crate::Scene=serde_json::from_value(serde_json::json!({"version":2,"images":{"plate":{"file":"plate.mp4"}},"nodes":[{"type":"camera"}],
            "animation":[{"loop":true,"tracks":[{"path":["images","plate","time"],"keys":[[0,2],[1,1],[2,1],[3,0]]}]}]}))?;
        let plan=crate::animation::Plan::compile(&scene,Path::new("."))?;
        for (time,expected) in [(0.,2.),(0.5,1.5),(1.5,1.),(2.5,0.5),(3.5,1.5)] {
            let mut samples=plan.samples();plan.sample_into(time,&mut samples);
            assert!((plan.image_time(&samples,"plate",time)-expected).abs()<1e-5);
        }
        Ok(())
    }
    #[test]
    fn indexed_video_seeks_preserve_frames_and_bound_reverse_work() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-footage-index-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let path=root.join("long.mp4");
        let status=Command::new("ffmpeg").args(["-v","error","-y","-f","lavfi","-i","testsrc2=size=16x16:rate=30:duration=20",
            "-c:v","libx264","-g","30","-keyint_min","30","-sc_threshold","0","-bf","3","-output_ts_offset","7"]).arg(&path).status()?;
        ensure!(status.success(),"make long-GOP video fixture");
        let expected=Command::new("ffmpeg").args(["-v","error","-i"]).arg(&path)
            .args(["-fps_mode","passthrough","-f","rawvideo","-pix_fmt","rgba64le","pipe:1"]).output()?;
        ensure!(expected.status.success(),"decode video reference");
        let mut source=Video::open(&path)?;
        let mut requests:Vec<_>=(500..600).rev().collect();
        requests.extend([5,590,60,450,300,0,599,599,10]);
        for index in requests {
            let before=source.decoded;
            let actual=source.frame(source.times[index],32)?.to_rgba16();
            let bytes:Vec<_>=actual.as_raw().iter().flat_map(|v|v.to_le_bytes()).collect();
            assert_eq!(bytes,&expected.stdout[index*16*16*8..(index+1)*16*16*8],"seek frame {index}");
            assert!(source.decoded-before<=30,"seek {index} decoded {} frames",source.decoded-before);
            assert!(source.frames.len()<=crate::constants::FOOTAGE_DECODE_FRAMES);
        }
        drop(source);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn video_timestamps_hold_reverse_and_autorotate() -> Result<()> {
        let root=std::env::temp_dir().join(format!("vork-footage-video-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        for (name,color) in [("red",[255,0,0]),("green",[0,255,0]),("blue",[0,0,255])] {
            image::RgbImage::from_pixel(8,4,image::Rgb(color)).save(root.join(format!("{name}.png")))?;
        }
        std::fs::write(root.join("frames.txt"),"file 'red.png'\nduration 0.04\nfile 'green.png'\nduration 0.16\nfile 'blue.png'\nduration 0.08\nfile 'blue.png'\n")?;
        let video=root.join("vfr.mp4");
        let status=Command::new("ffmpeg").current_dir(&root).args(["-v","error","-y","-f","concat","-safe","0","-i","frames.txt","-fps_mode","vfr","-c:v","libx264","-pix_fmt","yuv444p"]).arg(&video).status()?;
        ensure!(status.success(),"make VFR video fixture");
        let mut source=Video::open(&video)?;
        source.frame(0.,16)?;source.frame(0.04,16)?;
        let next=source.session.as_ref().unwrap().next;
        source.frame(0.,16)?;source.frame(0.04,16)?;
        assert_eq!(source.session.as_ref().unwrap().next,next,"shuffled shutter samples retain the decoder session");
        assert!(source.times.windows(3).any(|t| (t[1]-t[0]-(t[2]-t[1])).abs()>0.01));
        for (time,channel) in [(0.,0),(0.039,0),(0.04,1),(0.08,1),(0.19,1),(10.,2),(0.,0)] {
            let pixel=source.frame(time,16)?.to_rgb8()[(0,0)];
            assert!(pixel[channel]>240,"at {time}: {pixel:?}");
        }
        let offset=root.join("offset.mp4");
        let status=Command::new("ffmpeg").args(["-v","error","-y","-itsoffset","3","-i"]).arg(&video).args(["-c","copy"]).arg(&offset).status()?;
        ensure!(status.success(),"make edit-list video fixture");
        let mut shifted=Video::open(&offset)?;
        assert_eq!(shifted.times[0],0.);
        assert!(shifted.frame(0.08,16)?.to_rgb8()[(0,0)][1]>240);
        drop(shifted);
        let rotated=root.join("rotated.mp4");
        let status=Command::new("ffmpeg").args(["-v","error","-y","-display_rotation","90","-i"]).arg(&video).args(["-c","copy"]).arg(&rotated).status()?;
        ensure!(status.success(),"make rotated video fixture");
        let mut rotated=Video::open(&rotated)?;
        let frame=rotated.frame(0.,16)?;assert_eq!((frame.width(),frame.height()),(4,8));
        drop(rotated);drop(source);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn exr_sequence_clock_uses_first_header_and_rejects_invalid_rates() -> Result<()> {
        use exr::prelude::*;
        let root=std::env::temp_dir().join(format!("vork-exr-clock-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let write=|index,rate| -> Result<()> {
            let attributes=LayerAttributes {frames_per_second:rate,..Default::default()};
            let channels=SpecificChannels::rgb(|_:Vec2<usize>|(index as f32,0f32,0f32));
            Image::from_layer(Layer::new((1,1),attributes,Encoding::SMALL_LOSSLESS,channels)).write().to_file(root.join(format!("{index:04}.exr")))?;
            Ok(())
        };
        // A later header cannot change the sequence's clock.
        write(2,Some((120,1)))?;
        for rate in [Some((2,1)),None] {
            write(1,rate)?;
            let mut source=Source::open(&root.join("%04d.exr"))?;
            let fps=if rate.is_some() {2.} else {24.};
            assert_eq!(source.timeline(24.)?,vec![0.,1./fps]);
            assert_eq!(source.frame_index(0.04,24.),0);
            assert_eq!(source.frame_index(0.04,48.),usize::from(rate.is_none()));
            for render_fps in [24.,48.] {
                for time in [-1.,0.,0.04,0.5,100.,0.] {
                    let expected=1.+source.frame_index(time,render_fps) as f32;
                    assert_eq!(source.frame(time,render_fps,16)?.to_rgb32f()[(0,0)][0],expected);
                }
                for (index,time) in source.timeline(render_fps)?.into_iter().enumerate() {assert_eq!(source.frame_index(time,render_fps),index);}
            }
        }
        for rate in [(0,1),(1,0),(-1,1)] {
            write(1,Some(rate))?;
            let error=Source::open(&root.join("%04d.exr")).err().context("invalid EXR rate accepted")?;
            assert!(error.to_string().contains("framesPerSecond must be positive"),"{error:#}");
        }
        std::fs::remove_dir_all(root)?;Ok(())
    }

    #[test]
    fn sequences_start_at_first_disk_frame_hold_ends_and_reject_holes() -> Result<()> {
        let root = std::env::temp_dir().join(format!("vork-footage-sequence-{}",std::process::id()));
        std::fs::create_dir_all(&root)?;
        let dir = root.as_path();
        for n in [1001,1002] {
            image::RgbaImage::from_pixel(1,1,image::Rgba([(n-1000) as u8,0,0,255])).save(dir.join(format!("{n:04}.png")))?;
        }
        let mut source = Source::open(&dir.join("%04d.png"))?;
        for (time,expected) in [(-1.,1),(0.,1),(0.041,1),(1./24.,2),(100.,2)] {
            assert_eq!(source.frame(time,24.,16)?.to_rgba8()[ (0,0) ][0],expected);
        }
        std::fs::rename(dir.join("1002.png"),dir.join("1004.png"))?;
        let error = Source::open(&dir.join("%04d.png")).err().unwrap().to_string();
        assert!(error.contains("1002..1003"),"{error}");
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}

/// One resident texture, updated by ordered encoder copies rather than queue
/// writes so several shutter samples can coexist in one command submission.
pub(crate) struct Installed {
    pub name: String,
    pub time: Option<f32>,
    pub per_sample: bool,
    pub final_compositor: bool,
    pub view: wgpu::TextureView,
    source: std::sync::Mutex<Source>,
    texture: wgpu::Texture,
    mips: crate::materials::MipChain,
    srgb: bool,
}
impl Installed {
    /// End external input work while the completed shot still owns GPU resources.
    pub(crate) fn finish(&mut self) {
        if let Source::Video(video) = self.source.get_mut().unwrap() {
            video.session = None; // Session kills and reaps before closing its reader.
        }
    }
    pub(crate) fn new(r:&crate::Renderer,name:&str,path:&Path,time:Option<f32>,srgb:bool,fps:f32)->Result<Self> {
        let mut source=Source::open_gpu(path,r)?;
        let frame=source.frame(time.unwrap_or(0.),fps,r.device.limits().max_texture_dimension_2d)?;
        let size=wgpu::Extent3d {width:frame.width(),height:frame.height(),depth_or_array_layers:1};
        let texture=crate::gpu_memory::texture(&r.device,&wgpu::TextureDescriptor {
            label:Some("footage image"),size,mip_level_count:32-size.width.max(size.height).leading_zeros(),sample_count:1,
            dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba16Float,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:&[],
        });
        let (pipelines,sampler)=crate::materials::mip_resources(r);
        let mips=crate::materials::MipChain::new(&r.device,&texture,None,&pipelines[2],sampler);
        Ok(Self {name:name.into(),time,per_sample:true,final_compositor:false,view:texture.create_view(&Default::default()),texture,mips,source:std::sync::Mutex::new(source),srgb})
    }
    pub(crate) fn frame_index(&self,time:f32,fps:f32)->Result<usize> {
        ensure!(time.is_finite(),"image {}: time must be finite",self.name);
        Ok(self.source.lock().unwrap().frame_index(time,fps))
    }
    pub(crate) fn encode(&self,r:&crate::Renderer,encoder:&mut wgpu::CommandEncoder,time:f32,fps:f32)->Result<()> {
        let frame=self.source.lock().unwrap().frame(time,fps,r.device.limits().max_texture_dimension_2d)?;
        ensure!(frame.width()==self.texture.width() && frame.height()==self.texture.height(),"image {}: media frame dimensions changed",self.name);
        let rgba=frame.into_rgba32f();
        let pitch=(self.texture.width()*8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)*wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let mut data=vec![0u8;pitch as usize*self.texture.height() as usize];
        for (row,pixels) in rgba.as_raw().chunks_exact(self.texture.width() as usize*4).enumerate() {
            for (i,&value) in pixels.iter().enumerate() {
                let value=if self.srgb && i%4!=3 {crate::texture_source::decode_srgb(value)} else {value};
                let at=row*pitch as usize+i*2;
                data[at..at+2].copy_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
            }
        }
        use wgpu::util::DeviceExt;
        let staging=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("footage upload"),contents:&data,usage:wgpu::BufferUsages::COPY_SRC});
        encoder.copy_buffer_to_texture(wgpu::TexelCopyBufferInfo {buffer:&staging,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(pitch),rows_per_image:Some(self.texture.height())}},self.texture.as_image_copy(),self.texture.size());
        self.mips.encode(encoder,None);
        Ok(())
    }
}
