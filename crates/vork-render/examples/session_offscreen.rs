//! Bounded headless host and frame-budget receipt. Pixel download occurs only
//! after measurement, when --png is requested. This is not display latency.
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, time::Instant};
use vork_render::session::Session;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().context("usage: session_offscreen SCENE [--profile PROFILE] [--frames N] [--resolution WxH] [--samples N] [--out REPORT] [--png IMAGE]")?);
    let mut profile = "std/previs".to_string(); let mut frames = 120_u32;
    let mut settings = serde_json::Map::new(); let mut resolution = None; let mut out = None; let mut png = None;
    while let Some(arg) = args.next() {
        let value = args.next().with_context(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--profile" => profile = value,
            "--frames" => frames = value.parse()?,
            "--resolution" => {
                let (w,h) = value.split_once('x').context("resolution must be WxH")?;
                resolution = Some([w.parse::<u32>()?,h.parse::<u32>()?]);
            },
            "--samples" => { settings.insert("samples".into(), serde_json::json!(value.parse::<u32>()?)); },
            "--out" => out = Some(PathBuf::from(value)),
            "--png" => png = Some(PathBuf::from(value)),
            _ => anyhow::bail!("unknown argument {arg}"),
        }
    }
    ensure!(frames > 8 && frames <= 3600, "--frames must be 9..3600; first eight frames are warmup");
    let start = Instant::now();
    let mut scene = vork_render::load_authored(&path)?;
    if let Some(resolution) = resolution { scene.render.resolution = resolution; }
    let profile = vork_scene::Profile::load(&profile)?;
    scene.apply_settings(profile.resolve(&settings)?, &profile.name)?;
    let fps = scene.render.fps.as_f32();
    let mut session = Session::prepare(vork_render::Renderer::new()?, scene, path.parent().unwrap_or(std::path::Path::new(".")))?;
    let ready_ms = start.elapsed().as_secs_f64()*1000.;
    let pipeline_count = session.renderer().pipeline_timings().len();
    let mut rows = Vec::new(); let mut last = None;
    for index in 0..frames {
        let start = Instant::now();
        let frame = session.frame(index as f32/fps)?;
        let wall = start.elapsed().as_secs_f64()*1000.;
        last = Some(frame.image.clone());
        rows.push(serde_json::json!({"frame":index,"wall_ms":wall,
            "gpu_envelope_ms":frame.timing.gpu_elapsed_ms,"cpu_prepare_ms":frame.timing.cpu_prepare_ms,
            "timing_source":frame.timing.timing_source,"sample_count":frame.timing.sample_count,
            "dropped_light_samples":frame.timing.dropped_light_samples,"nonfinite":frame.timing.nonfinite}));
    }
    let p95 = |key: &str| {
        let mut values: Vec<_> = rows[8..].iter().map(|row| row[key].as_f64().unwrap()).collect();
        values.sort_by(f64::total_cmp);
        values[(values.len()*95).div_ceil(100)-1]
    };
    let wall = p95("wall_ms");
    let report = serde_json::json!({"gpu":session.renderer().adapter_info.name,
        "backend":format!("{:?}",session.renderer().adapter_info.backend),"driver":session.renderer().adapter_info.driver_info,
        "resolution":session.resolution(),"profile":profile.name,"resolved_settings":session.render().settings,"ready_ms":ready_ms,"warmup_frames":8,
        "wall_p95_ms":wall,"gpu_envelope_p95_ms":p95("gpu_envelope_ms"),"fits_30hz":wall<=1000./30.,"fits_60hz":wall<=1000./60.,
        "pipeline_creations_during_frames":session.renderer().pipeline_timings().len()-pipeline_count,
        "measurement":"offscreen single-flight host wall; excludes presentation and gameplay; diagnostic readback included",
        "frames":rows});
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(out) = out { std::fs::write(out, &bytes)?; } else { println!("{}",String::from_utf8(bytes)?); }
    if let Some(path) = png { save_png(session.renderer(), &last.unwrap(), &path)?; }
    Ok(())
}

fn save_png(r: &vork_render::Renderer, image: &wgpu::Texture, path: &std::path::Path) -> Result<()> {
    let [w,h] = [image.width(), image.height()]; let pitch = (w*4).div_ceil(256)*256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor { label:Some("offscreen validation download"),size:u64::from(pitch)*u64::from(h),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(image.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer:&buffer,
        layout:wgpu::TexelCopyBufferLayout { offset:0, bytes_per_row:Some(pitch), rows_per_image:Some(h) } }, image.size());
    let submission = r.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read,move |result| {let _=tx.send(result);});
    r.device.poll(wgpu::PollType::Wait { submission_index:Some(submission), timeout:Some(std::time::Duration::from_secs(30)) })?;
    rx.recv_timeout(std::time::Duration::from_secs(30))??;
    let data=buffer.slice(..).get_mapped_range();
    let mut rgba:Vec<u8>=data.chunks(pitch as usize).flat_map(|row| row[..w as usize*4].iter().copied()).collect();
    if image.format()==wgpu::TextureFormat::Bgra8Unorm {for pixel in rgba.chunks_exact_mut(4) {pixel.swap(0,2);}}
    let mut encoder=png::Encoder::new(std::fs::File::create(path)?,w,h);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
