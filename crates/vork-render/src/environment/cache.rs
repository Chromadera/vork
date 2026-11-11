//! Content-addressed, checksummed final HDR integrals. Full source radiance stays
//! at input resolution; only the small filtered lighting set is stored here.
use super::*;

pub(super) fn path(
    r: &Renderer,
    hash: blake3::Hash,
) -> PathBuf {
    let mut key=blake3::Hasher::new();key.update(hash.as_bytes());
    key.update(FILTER_SHADER.as_bytes());
    key.update(REDUCE_SHADER.as_bytes());
    key.update(include_bytes!("../../shaders/environment_evaluated.wgsl"));
    key.update(&crate::constants::ENVIRONMENT_PROPOSAL_MAX.to_le_bytes());
    key.update(&crate::constants::ENVIRONMENT_LEVELS.to_le_bytes());
    key.update(&crate::constants::ENVIRONMENT_WIDTH.to_le_bytes());
    for size in crate::constants::DIFFUSE_SIZE {
        key.update(&size.to_le_bytes());
    }
    r.disk_cache.path("hdr-filtered", 9, key.finalize())
}

pub(super) struct Filtered {
    pub sh: [[f64; 3]; 4],
    images: Vec<Vec<u8>>,
}

fn shapes() -> Vec<(u32, u32)> {
    let width = crate::constants::ENVIRONMENT_WIDTH;
    (0..crate::constants::ENVIRONMENT_LEVELS)
        .map(|mip| ((width >> mip).max(1), ((width / 2) >> mip).max(1)))
        .chain([tuple(crate::constants::DIFFUSE_SIZE)])
        .collect()
}
fn tuple(v: [u32; 2]) -> (u32, u32) {
    (v[0], v[1])
}

impl Filtered {
    pub fn read(path: &Path) -> Option<Self> {
        let payload = disk_cache::read(path)?;
        let length = u32::from_le_bytes(payload.get(..4)?.try_into().ok()?) as usize;
        let metadata: serde_json::Value =
            serde_json::from_slice(payload.get(4..4usize.checked_add(length)?)?).ok()?;
        let sh = serde_json::from_value(metadata.get("sh")?.clone()).ok()?;
        let mut offset = 4 + length;
        let mut images = Vec::new();
        for (w, h) in shapes() {
            let end = offset.checked_add((w * h * 8) as usize)?;
            images.push(payload.get(offset..end)?.to_vec());
            offset = end;
        }
        if offset != payload.len() {
            return None;
        }
        Some(Self { sh, images })
    }
    pub fn write(&self, path: &Path) {
        let Ok(metadata) =
            serde_json::to_vec(&serde_json::json!({ "sh": self.sh }))
        else {
            return;
        };
        let mut payload = Vec::with_capacity(4+metadata.len()+self.images.iter().map(Vec::len).sum::<usize>());
        payload.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        payload.extend_from_slice(&metadata);
        for image in &self.images {
            payload.extend_from_slice(image);
        }
        let _ = disk_cache::write(path, &payload);
    }
    pub fn upload(&self, r: &Renderer, specular: &wgpu::Texture, diffuse: &wgpu::Texture) {
        for (i, ((w, h), image)) in shapes().into_iter().zip(&self.images).enumerate() {
            let target = if i < crate::constants::ENVIRONMENT_LEVELS as usize {
                specular
            } else {
                diffuse
            };
            r.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: if i < crate::constants::ENVIRONMENT_LEVELS as usize {
                        i as u32
                    } else {
                        0
                    },
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                image,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 8),
                    rows_per_image: Some(h),
                },
                extent(w, h),
            );
        }
    }
    pub fn capture(
        r: &Renderer,
        specular: &wgpu::Texture,
        diffuse: &wgpu::Texture,
        sh: [glam::DVec3; 4],
        path:PathBuf,
    ) {
        let shapes = shapes();
        // Reserve padded GPU readbacks, copied images and serialization together.
        // 4 KiB covers the fixed SH metadata and collection headers.
        let bytes=constants::CACHE_CAPTURE_METADATA_BYTES
            +(shapes.len()+3)*path.as_os_str().as_encoded_bytes().len()+shapes.iter().map(|&(w,h)| {
            let stride=(w*8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)*wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            (stride as usize+2*w as usize*8)*h as usize
        }).sum::<usize>();
        let Some(reservation)=disk_cache::reserve_persistence(&path,bytes) else {return;};
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let mut buffers = Vec::new();
        for (i, &(w, h)) in shapes.iter().enumerate() {
            let stride = (w * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = crate::gpu_memory::buffer(&r.device, &wgpu::BufferDescriptor {
                label: Some("filtered HDR cache readback"),
                size: stride as u64 * h as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: if i < crate::constants::ENVIRONMENT_LEVELS as usize {
                        specular
                    } else {
                        diffuse
                    },
                    mip_level: if i < crate::constants::ENVIRONMENT_LEVELS as usize {
                        i as u32
                    } else {
                        0
                    },
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(h),
                    },
                },
                extent(w, h),
            );
            buffers.push((buffer, stride));
        }
        crate::command_finish::Segments::submit(&r.queue, encoder);
        struct Pending {images:Vec<Option<Vec<u8>>>,failed:bool,reservation:std::sync::Arc<disk_cache::PersistenceReservation>}
        let pending=std::sync::Arc::new(std::sync::Mutex::new(Pending {images:vec![None;buffers.len()],failed:false,reservation:std::sync::Arc::new(reservation)}));
        for (index,((buffer,stride),(w,_))) in buffers.into_iter().zip(shapes).enumerate() {
            let pending=pending.clone();let mapped_buffer=buffer.clone();let path=path.clone();
            buffer.map_async(wgpu::MapMode::Read,..,move |result| {
                let mut pending=pending.lock().unwrap();
                if result.is_err() {pending.failed=true;return;}
                let mapped=mapped_buffer.get_mapped_range(..);
                let mut image=Vec::with_capacity(mapped.len()/stride as usize*(w*8) as usize);
                for row in mapped.chunks_exact(stride as usize) {image.extend_from_slice(&row[..(w*8) as usize]);}
                drop(mapped);mapped_buffer.unmap();pending.images[index]=Some(image);
                if !pending.failed && pending.images.iter().all(Option::is_some) {
                    let cached=Filtered {sh:sh.map(|v|v.to_array()),images:pending.images.iter_mut().map(|image|image.take().unwrap()).collect()};
                    let reservation=pending.reservation.clone();
                    drop(pending);reservation.publish(move ||cached.write(&path));
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore="requires real GPU"]
    fn held_store_lock_does_not_block_gpu_collection() -> Result<()> {
        let r=Renderer::new()?;
        let scene=vork_scene::v2::from_test_value(serde_json::json!({"version":2,
            "render":{"resolution":[8,8],"settings":{"samples":1,"denoiser":null}},
            "nodes":[{"type":"camera"}]}))?;
        let installed=r.install(scene,Path::new("."))?;
        let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
        let texture=|size,mip_level_count|crate::gpu_memory::texture(&r.device,&wgpu::TextureDescriptor {
            label:Some("cache lock regression"),size,mip_level_count,sample_count:1,
            dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba16Float,
            usage:wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::COPY_DST,view_formats:&[],
        });
        let w=constants::ENVIRONMENT_WIDTH;
        let specular=texture(extent(w,w/2),constants::ENVIRONMENT_LEVELS);
        let [w,h]=constants::DIFFUSE_SIZE;
        let diffuse=texture(extent(w,h),1);
        let root=std::env::temp_dir().join(format!("vork-cache-lock-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        std::fs::create_dir(&root)?;
        let store=disk_cache::Store::for_test(root.clone());
        let path=store.path("hdr-filtered",9,blake3::hash(b"held-lock"));
        let lock=disk_cache::lock_store(&root)?;
        let expired=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        std::thread::scope(|scope| -> Result<()> {
            let (release,wait)=std::sync::mpsc::channel();
            let expired=expired.clone();
            scope.spawn(move || {
                if matches!(wait.recv_timeout(std::time::Duration::from_secs(10)),Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
                    expired.store(true,std::sync::atomic::Ordering::SeqCst);
                }
                drop(lock);
            });
            Filtered::capture(&r,&specular,&diffuse,[glam::DVec3::ZERO;4],path.clone());
            // Service every map callback, then submit and collect a real frame
            // while the cross-process publication lock is still unavailable.
            r.wait(None)?;
            let first=disk_cache::persistence_bytes();
            assert!(first>0);
            for _ in 0..32 {Filtered::capture(&r,&specular,&diffuse,[glam::DVec3::ZERO;4],path.clone());}
            r.wait(None)?;
            assert_eq!(disk_cache::persistence_bytes(),first,"duplicate content must coalesce");
            for i in 0u32..64 {
                let next=store.path("hdr-filtered",9,blake3::hash(&i.to_le_bytes()));
                Filtered::capture(&r,&specular,&diffuse,[glam::DVec3::ZERO;4],next);
                r.wait(None)?;
                assert!(disk_cache::persistence_bytes()<=constants::CACHE_PERSISTENCE_BYTES);
            }
            assert!(disk_cache::reserve_persistence(&root.join("overflow"),first).is_none(),"full queue must decline without waiting");
            frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
            assert!(frame.collect(&r)?.linear_rgba.unwrap().iter().flatten().all(|v|v.is_finite()));
            assert!(!path.exists(),"publication acquired a held store lock");
            let _=release.send(());
            Ok(())
        })?;
        assert!(!expired.load(std::sync::atomic::Ordering::SeqCst),"GPU polling/collection waited for disk persistence");
        disk_cache::wait_persistence()?;
        assert_eq!(disk_cache::persistence_bytes(),0,"publication must release every reservation");
        let recovery=store.path("hdr-filtered",9,blake3::hash(b"recovery"));
        Filtered::capture(&r,&specular,&diffuse,[glam::DVec3::ZERO;4],recovery.clone());
        r.wait(None)?;disk_cache::wait_persistence()?;
        assert!(Filtered::read(&recovery).is_some(),"admission must recover after unlock");
        let cached=Filtered::read(&path).context("cache publication after unlock")?;
        assert_eq!(cached.sh,[[0.;3];4]);
        assert!(cached.images.iter().flatten().all(|v|*v==0));
        std::fs::remove_dir_all(root)?;
        r.errors.check()?;Ok(())
    }
}
