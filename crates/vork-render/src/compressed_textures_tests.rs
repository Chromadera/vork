use super::*;

#[test]
fn skipped_work_preserves_gpu_bytes() -> Result<()> {
    for (w, h) in [(4, 4), (16, 8), (32, 64)] {
        for alpha in [false, true] {
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let x = i % w;
                    let y = i / w;
                    [
                        (x * 79 + y * 11) as u8,
                        (x * y * 17) as u8,
                        (i * 137) as u8,
                        if alpha { (i * 53) as u8 } else { 255 },
                    ]
                })
                .collect();
            for srgb in [false, true] {
                for astc in [false, true] {
                    assert_eq!(
                        encode_basis(&pixels, w, h, srgb, astc, false)?,
                        encode_basis(&pixels, w, h, srgb, astc, true)?,
                        "{w}x{h}, alpha={alpha}, srgb={srgb}, astc={astc}"
                    );
                }
            }
        }
    }
    for color in [
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        [19, 83, 207, 0],
        [19, 83, 207, 143],
    ] {
        let pixels = color.repeat(16);
        for srgb in [false, true] {
            for astc in [false, true] {
                assert_eq!(
                    encode_basis(&pixels, 4, 4, srgb, astc, false)?,
                    encode_basis(&pixels, 4, 4, srgb, astc, true)?
                );
            }
        }
    }
    Ok(())
}

fn direct_quality(im: &image::RgbaImage, srgb: bool) -> Result<serde_json::Value> {
    let start = Instant::now();
    let direct = basis_container(
        &im,
        im.width(),
        im.height(),
        srgb,
        true,
        BasisTextureFormat::AstcLdr4x4,
    )?;
    let direct_s = start.elapsed().as_secs_f64();
    let baseline = basis_container(
        &im,
        im.width(),
        im.height(),
        srgb,
        true,
        BasisTextureFormat::UastcLdr4x4,
    )?;
    let decode = |container: &[u8]| -> Result<Vec<u8>> {
        Ok(BasisuTranscoder::new(
            container,
            SupportedTextureCompression::empty(),
            ChannelType::Rgba,
        )?
        .transcode(None, Some(srgb))?
        .data)
    };
    let psnr = |decoded: &[u8], channels: usize| {
        let sum: f64 = im
            .as_raw()
            .chunks_exact(4)
            .zip(decoded.chunks_exact(4))
            .map(|(a, b)| {
                (0..channels)
                    .map(|c| (f64::from(a[c]) - f64::from(b[c])).powi(2))
                    .sum::<f64>()
            })
            .sum();
        10. * (255.0_f64.powi(2) * f64::from(im.width()) * f64::from(im.height()) * channels as f64
            / sum)
            .log10()
    };
    let baseline = decode(&baseline)?;
    let direct = decode(&direct)?;
    Ok(
        serde_json::json!({"direct_s":direct_s,"baseline_rgb_psnr":psnr(&baseline,3),"direct_rgb_psnr":psnr(&direct,3),"baseline_rgba_psnr":psnr(&baseline,4),"direct_rgba_psnr":psnr(&direct,4)}),
    )
}

#[test]
#[ignore = "parallel direct ASTC quality evaluation; set VORK_ASTC_CORPUS"]
fn direct_corpus() -> Result<()> {
    let paths: Vec<std::path::PathBuf> =
        serde_json::from_slice(&std::fs::read(std::env::var("VORK_ASTC_CORPUS")?)?)?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let rows = std::sync::Mutex::new(Vec::new());
    let count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(paths.len());
    std::thread::scope(|scope| -> Result<()> {
        let joins: Vec<_> = (0..count).map(|_| scope.spawn(|| -> Result<()> {
            loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(path) = paths.get(i) else { break };
                let im = image::open(path)?.to_rgba8();
                for srgb in [false, true] {
                    let row = serde_json::json!({"path":path,"srgb":srgb,"quality":direct_quality(&im, srgb)?});
                    eprintln!("{row}");
                    rows.lock().unwrap().push(row);
                }
            }
            Ok(())
        })).collect();
        for join in joins {
            join.join().unwrap()?;
        }
        Ok(())
    })?;
    if let Ok(output) = std::env::var("VORK_ASTC_RESULTS") {
        std::fs::write(
            output,
            serde_json::to_vec_pretty(&rows.into_inner().unwrap())?,
        )?;
    }
    Ok(())
}

/// VORK_ASTC_CORPUS is a JSON array of image paths. No GPU required.
#[test]
#[ignore = "corpus encode timing; set VORK_ASTC_CORPUS"]
fn corpus() -> Result<()> {
    let paths: Vec<std::path::PathBuf> =
        serde_json::from_slice(&std::fs::read(std::env::var("VORK_ASTC_CORPUS")?)?)?;
    let mut rows = Vec::new();
    for path in paths {
        let im = image::open(&path)?.to_rgba8();
        for srgb in [false, true] {
            let start = Instant::now();
            let baseline = encode_basis(&im, im.width(), im.height(), srgb, true, false)?;
            let before = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let candidate = encode_basis(&im, im.width(), im.height(), srgb, true, true)?;
            let after = start.elapsed().as_secs_f64();
            ensure!(
                baseline == candidate,
                "ASTC bytes differ for {} srgb={srgb}",
                path.display()
            );
            let quality = if std::env::var_os("VORK_ASTC_DIRECT").is_some() {
                direct_quality(&im, srgb)?
            } else {
                serde_json::Value::Null
            };
            let row = serde_json::json!({"path":path,"width":im.width(),"height":im.height(),"srgb":srgb,"baseline_s":before,"candidate_s":after,"quality":quality,"bytes":baseline.len(),"hash":blake3::hash(&candidate).to_hex().to_string()});
            eprintln!("{row}");
            rows.push(row);
        }
    }
    // Exercise the same across-texture scheduling as installation, without a GPU.
    let images: Vec<_> = rows
        .iter()
        .map(|row| {
            let path = row["path"].as_str().unwrap();
            Ok((
                image::open(path)?.to_rgba8(),
                row["srgb"].as_bool().unwrap(),
                row["hash"].as_str().unwrap().to_owned(),
            ))
        })
        .collect::<Result<_>>()?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(images.len());
    let start = Instant::now();
    std::thread::scope(|scope| -> Result<()> {
        let joins: Vec<_> = (0..count)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((im, srgb, hash)) = images.get(i) else {
                            break;
                        };
                        let data = encode_basis(im, im.width(), im.height(), *srgb, true, true)?;
                        ensure!(
                            blake3::hash(&data).to_hex().as_str() == hash,
                            "parallel encoding changed bytes"
                        );
                    }
                    Ok(())
                })
            })
            .collect();
        for join in joins {
            join.join().unwrap()?;
        }
        Ok(())
    })?;
    eprintln!(
        "batch: {} textures, {count} workers, {:.6}s",
        images.len(),
        start.elapsed().as_secs_f64()
    );
    if let Ok(output) = std::env::var("VORK_ASTC_RESULTS") {
        std::fs::write(output, serde_json::to_vec_pretty(&rows)?)?;
    }
    Ok(())
}

/// Match actual glTF tap/color-space jobs to the serial corpus measurements.
/// VORK_ASTC_JOBS is an array of {path,srgb}; duplicate decoded inputs encode once.
#[test]
#[ignore = "scene encode timing; requires corpus results and VORK_ASTC_JOBS"]
fn scene_batch() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Input {
        path: std::path::PathBuf,
        srgb: bool,
    }
    let jobs: Vec<Input> =
        serde_json::from_slice(&std::fs::read(std::env::var("VORK_ASTC_JOBS")?)?)?;
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(std::env::var(
        "VORK_ASTC_CORPUS_RESULTS",
    )?)?)?;
    let mut measured = std::collections::HashMap::new();
    for row in rows {
        let im = image::open(row["path"].as_str().unwrap())?.to_rgba8();
        measured.insert(
            (
                blake3::hash(&im),
                im.width(),
                im.height(),
                row["srgb"].as_bool().unwrap(),
            ),
            row,
        );
    }
    let mut seen = std::collections::HashSet::new();
    let mut images = Vec::new();
    let mut baseline = 0.;
    let mut serial_candidate = 0.;
    for job in jobs {
        let im = image::open(job.path)?.to_rgba8();
        let key = (blake3::hash(&im), im.width(), im.height(), job.srgb);
        if !seen.insert(key) {
            continue;
        }
        let row = measured
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("scene texture absent from corpus"))?;
        baseline += row["baseline_s"].as_f64().unwrap();
        serial_candidate += row["candidate_s"].as_f64().unwrap();
        images.push((im, job.srgb, row["hash"].as_str().unwrap().to_owned()));
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let count = std::env::var("VORK_ASTC_WORKERS")
        .ok()
        .map(|n| n.parse())
        .transpose()?
        .unwrap_or(std::thread::available_parallelism().map_or(1, usize::from))
        .min(images.len());
    ensure!(
        count > 0,
        "scene timing needs at least one worker and image"
    );
    let start = Instant::now();
    std::thread::scope(|scope| -> Result<()> {
        let joins: Vec<_> = (0..count)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((im, srgb, hash)) = images.get(i) else {
                            break;
                        };
                        let data = encode_basis(im, im.width(), im.height(), *srgb, true, true)?;
                        ensure!(
                            blake3::hash(&data).to_hex().as_str() == hash,
                            "scene bytes changed"
                        );
                    }
                    Ok(())
                })
            })
            .collect();
        for join in joins {
            join.join().unwrap()?;
        }
        Ok(())
    })?;
    let row = serde_json::json!({"textures":images.len(),"workers":count,"baseline_serial_sum_s":baseline,"candidate_serial_sum_s":serial_candidate,"candidate_parallel_s":start.elapsed().as_secs_f64()});
    eprintln!("{row}");
    if let Ok(output) = std::env::var("VORK_ASTC_RESULTS") {
        std::fs::write(output, serde_json::to_vec_pretty(&row)?)?;
    }
    Ok(())
}
