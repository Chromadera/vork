//! NanoVDB bytes are opaque grid resources, never numeric vertex attributes.
use super::*;

pub(crate) struct Grid {
    pub(crate) words: Vec<u32>,
    pub(crate) bounds: (Vec3, Vec3),
}
impl Grid {
    pub(crate) fn load(buffer: &vork_scene::v2::Buffer, base: &Path) -> Result<Self> {
        use vork_scene::v2::BufferType;
        let name = buffer.grid.as_deref().context("grid name required")?;
        let path = base.join(buffer.file.as_deref().context("grid file required")?);
        let (kind, width): (&str, u32) = match buffer.element_type {
            BufferType::F32 => ("f32", 1),
            BufferType::Vec3 => ("vec3", 3),
            _ => bail!("grid values require f32 or vec3"),
        };
        ensure!(
            buffer.data.is_none() && buffer.offset == 0 && buffer.count.is_none(),
            "grid buffers require file and cannot have data/offset/count"
        );
        let bytes = std::fs::read(&path).with_context(|| format!("read grid {}",path.display()))?;
        if bytes.len() >= 736 && (&bytes[..8] == b"NanoVDB0" || &bytes[..8] == b"NanoVDB1") {
            let u32_at=|at| u32::from_le_bytes(bytes[at..at+4].try_into().unwrap());
            let u64_at=|at| u64::from_le_bytes(bytes[at..at+8].try_into().unwrap());
            ensure!(u64_at(32) == bytes.len() as u64 && bytes.len() % 32 == 0, "invalid raw NanoVDB size");
            ensure!(u32_at(28) == 1 && u32_at(636) == if width == 1 {1} else {6}, "grid count or voxel type mismatch");
            let stored_name=&bytes[40..296];
            let end=stored_name.iter().position(|&c| c==0).unwrap_or(stored_name.len());
            ensure!(&stored_name[..end] == name.as_bytes(), "raw NanoVDB grid name mismatch");
            let f64_at=|at| f64::from_le_bytes(bytes[at..at+8].try_into().unwrap()) as f32;
            let mut protocol=Vec::new();
            protocol.extend(b"VORKVDB1");
            protocol.extend(width.to_le_bytes());
            protocol.extend(u32::try_from(bytes.len())?.to_le_bytes());
            for i in 0..6 {protocol.extend(f64_at(560+i*8).to_le_bytes());}
            protocol.extend(bytes);
            return Self::decode(&protocol,width);
        }
        let helper = std::env::var_os("VORK_VDB").unwrap_or_else(|| "vork-vdb".into());
        let output = std::process::Command::new(helper)
            .arg(&path)
            .arg(name)
            .arg(kind)
            .output()
            .context("run vork-vdb loader; install tools/vdb or set VORK_VDB")?;
        ensure!(
            output.status.success(),
            "load grid {name:?} from {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        Self::decode(&output.stdout, width)
    }
    fn decode(bytes: &[u8], width: u32) -> Result<Self> {
        ensure!(
            bytes.len() >= 40 && &bytes[..8] == b"VORKVDB1",
            "invalid grid loader header"
        );
        let word =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        ensure!(word(8) == width, "grid voxel type differs from buffer.type");
        let length = usize::try_from(word(12))?;
        ensure!(
            length > 0 && length % 4 == 0 && bytes.len() - 40 == length,
            "invalid grid byte count"
        );
        let point = |offset| {
            Vec3::new(
                f32::from_bits(word(offset)),
                f32::from_bits(word(offset + 4)),
                f32::from_bits(word(offset + 8)),
            )
        };
        let bounds = (point(16), point(28));
        ensure!(
            bounds.0.is_finite() && bounds.1.is_finite() && bounds.0.cmplt(bounds.1).all(),
            "grid requires finite nonempty bounds"
        );
        let words: Vec<u32> = bytes[40..]
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        grid_maximum(&words,width)?;
        Ok(Self { words, bounds })
    }
}

/// Visit actual stored values, including inactive voxels and sparse background.
/// Do not trust optional NanoVDB min/max statistics (raw grids may omit them).
pub(crate) fn grid_maximum(words:&[u32], width:u32)->Result<f32> {
    fn word(w:&[u32],at:usize)->Result<u32> {ensure!(at%4==0,"unaligned grid address");w.get(at/4).copied().context("grid value address out of bounds")}
    fn offset(w:&[u32],base:usize,at:usize)->Result<usize> {
        let bits=u64::from(word(w,at)?) | (u64::from(word(w,at+4)?)<<32);
        let address=base.checked_add_signed(isize::try_from(bits as i64)?).context("grid child address overflow")?;
        ensure!(address< w.len()*4,"grid child address out of bounds");
        Ok(address)
    }
    fn value(w:&[u32],at:usize,width:u32,max:&mut f32)->Result<()> {
        for c in 0..width as usize {let v=f32::from_bits(word(w,at+c*4)?);ensure!(v.is_finite(),"grid contains nonfinite values");*max=max.max(v);}
        Ok(())
    }
    fn node(w:&[u32],at:usize,level:u32,width:u32,max:&mut f32)->Result<()> {
        let vector=width==3;
        if level==0 {
            let table=at+if vector {128}else{96};
            for i in 0..512 {value(w,table+i*width as usize*4,width,max)?;}
        } else {
            let (count,mask,table)=if level==2 {(32768,4128,8256)}else{(4096,544,1088)};
            let stride=if vector {16}else{8};
            for i in 0..count {
                let slot=at+table+i*stride;
                if word(w,at+mask+(i/32)*4)? & (1<<(i%32)) !=0 {node(w,offset(w,at,slot)?,level-1,width,max)?;}
                else {value(w,slot,width,max)?;}
            }
        }
        Ok(())
    }
    let root=offset(words,672,696)?;
    let mut maximum=f32::NEG_INFINITY;
    value(words,root+28,width,&mut maximum)?;
    let size=if width==3 {96}else{64};
    let count=word(words,root+24)? as usize;
    ensure!(count<=words.len()/8,"invalid grid tile count");
    for i in 0..count {
        let tile=root+size+i*32;
        let child=offset(words,root,tile+8)?;
        if child==root {value(words,tile+20,width,&mut maximum)?;}
        else {node(words,child,2,width,&mut maximum)?;}
    }
    Ok(maximum)
}


#[cfg(all(test, vork_private_tests))]
include!("vdb_tests_private.rs");

/// Closed bounds enter the normal geometry upload, motion and RT path.
pub(crate) fn bounds_geometry(bounds: (Vec3, Vec3)) -> crate::geometry::Geometry {
    let (lo, hi) = bounds;
    let positions: Vec<_> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ]
        })
        .collect();
    let indices = vec![
        0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6, 2,
        1, 3, 7, 1, 7, 5,
    ];
    crate::geometry::Geometry::channels(
        positions,
        vec![[0.; 3]; 8],
        vec![[[0.; 2]; 2]; 8],
        indices,
        0,
    )
}
