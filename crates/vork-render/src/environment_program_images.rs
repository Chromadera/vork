//! Ordinary image bindings for standalone environment evaluation. File/media
//! decoding and clocks are shared with the other program stages.
use super::*;
use vork_scene::v2;

pub(super) struct Images {
    sources: Vec<Source>,
    animation: crate::animation::Plan,
    fps: vork_time::Time,
    max_side: u32,
}
struct Source {
    name: String,
    definition: v2::Image,
    pixels: Vec<u32>,
    media: Option<std::sync::Mutex<crate::footage::Source>>,
    identity: blake3::Hash,
}
impl Images {
    pub(super) fn new(r: &Renderer, scene: &v2::Scene, base: &Path, layout: &vork_slang::InputLayout,
        values: &std::collections::BTreeMap<String, serde_json::Value>) -> Result<(Self, Vec<u32>)> {
        let mut sources=Vec::new();
        let words=layout.pack(values, |field,value| {
            let name=value.get("image").and_then(serde_json::Value::as_str).with_context(||format!("environment input {field}: expected {{image: name}}"))?;
            let definition=scene.images.get(name).with_context(||format!("unknown image {name}"))?.clone();
            let path=base.join(definition.file.as_ref().context("environment programs cannot sample rendered images")?);
            let media=crate::footage::is_media(&path).then(||crate::footage::Source::open_gpu(&path,r)).transpose()?.map(std::sync::Mutex::new);
            let identity=if let Some(media)=&media {media.lock().unwrap().identity()?} else {blake3::hash(&std::fs::read(&path)?)};
            let pixels=if media.is_none() {
                encode(&definition,crate::texture_source::decode(&path,&std::fs::read(&path)?,r.device.limits().max_texture_dimension_2d)?)
            } else {Vec::new()};
            let slot=sources.len() as u32;
            sources.push(Source {name:name.into(),definition,pixels,media,identity});
            Ok(slot)
        })?;
        let images=Self {sources,animation:crate::animation::Plan::compile_with_buffers(scene,base,None)?,fps:scene.render.fps,max_side:r.device.limits().max_texture_dimension_2d};
        Ok((images,words))
    }
    pub(super) fn count(&self)->usize {self.sources.len()}
    pub(super) fn install(&self,r:&Renderer,data:&[u32])->Result<Vec<wgpu::Texture>> {
        ensure!(self.sources.len()<=r.device.limits().max_sampled_textures_per_shader_stage as usize,"environment image inputs exceed this GPU's sampled texture bindings");
        let textures=(0..self.sources.len()).map(|k| {let at=data[k] as usize;crate::gpu_memory::texture(&r.device,&wgpu::TextureDescriptor {
            label:Some("environment program image float bits"),size:wgpu::Extent3d {width:data[at],height:data[at+1],depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba32Uint,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[],
        })}).collect::<Vec<_>>();
        self.upload(r,&textures,data)?;Ok(textures)
    }
    pub(super) fn upload(&self,r:&Renderer,textures:&[wgpu::Texture],data:&[u32])->Result<()> {
        for (k,texture) in textures.iter().enumerate() {
            let at=data[k] as usize;let (w,h)=(data[at],data[at+1]);
            ensure!(texture.width()==w && texture.height()==h,"environment footage dimensions must remain constant");
            r.queue.write_texture(texture.as_image_copy(),bytemuck::cast_slice(&data[at+5..at+5+(w*h*4) as usize]),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(w*16),rows_per_image:Some(h)},texture.size());
        }
        Ok(())
    }
    pub(super) fn shader(&self)->String {
        use std::fmt::Write;
        let mut shader=String::new();let mut loads=String::new();let mut sizes=String::new();let mut modes=String::new();
        let wrap=|w|match w {v2::Wrap::Repeat=>0,v2::Wrap::ClampToEdge=>1,v2::Wrap::MirroredRepeat=>2};
        for (k,source) in self.sources.iter().enumerate() {
            writeln!(shader,"@group(0) @binding({}) var environment_image_{k}:texture_2d<u32>;",k+3).unwrap();
            write!(loads,"case {k}u: {{return bitcast<vec4<f32>>(textureLoad(environment_image_{k},p,0));}}").unwrap();
            write!(sizes,"case {k}u: {{return vec2<i32>(textureDimensions(environment_image_{k}));}}").unwrap();
            write!(modes,"case {k}u: {{return vec3<u32>({}u,{}u,{}u);}}",wrap(source.definition.wrap_s),wrap(source.definition.wrap_t),u32::from(matches!(source.definition.mag_filter,v2::MagFilter::Nearest))).unwrap();
        }
        writeln!(shader,"fn environment_image_extent(slot:u32)->vec2<i32> {{switch slot {{{sizes} default: {{return vec2(1);}}}}}}\nfn environment_image_modes(slot:u32)->vec3<u32> {{switch slot {{{modes} default: {{return vec3(0u);}}}}}}\nfn environment_image_texel(slot:u32,p:vec2<i32>)->vec4<f32> {{switch slot {{{loads} default: {{return vec4(1.);}}}}}}").unwrap();
        shader.push_str(&sampling_shader());shader
    }
    pub(super) fn identity(&self)->blake3::Hash {let mut hash=blake3::Hasher::new();for source in &self.sources {hash.update(source.identity.as_bytes());hash.update(&serde_json::to_vec(&source.definition.time).expect("serializable image clock"));}hash.update(self.animation.input_identity().as_bytes());hash.finalize()}
    pub(super) fn size(&self,words:&[u32])->Option<[u32;2]> {
        (!self.sources.is_empty()).then(||self.sources.iter().enumerate().fold([1,1],|size,(k,_)| {let at=words[k] as usize;[size[0].max(words[at]),size[1].max(words[at+1])]}))
    }
    pub(super) fn animated(&self)->bool {self.sources.iter().any(|s|s.media.is_some())}
    pub(super) fn sample_key(&self,request:&vork_time::Request)->Result<Vec<u8>> {
        let time=request.numeric()?;
        let mut result=Vec::new();
        let mut samples=self.animation.samples();self.animation.sample_into(time,&mut samples);
        for s in &self.sources {if let Some(media)=&s.media {
            let time=request.exact()?;
            let base=s.definition.time.as_ref().map(|t|vork_time::exact::Germ::point(t.exact())).unwrap_or_else(||time.clone());
            let clock=self.animation.image_time_exact(&samples,&time,&s.name,base)?;
            result.extend_from_slice(&(media.lock().unwrap().frame_index_exact(&clock,self.fps)? as u64).to_le_bytes());
        }}
        Ok(result)
    }
    pub(super) fn words(&self,request:&vork_time::Request)->Result<Vec<u32>> {
        let time=request.numeric()?;
        let mut words=vec![0;self.sources.len().max(1)];
        let mut samples=self.animation.samples();self.animation.sample_into(time,&mut samples);
        for (k,s) in self.sources.iter().enumerate() {
            words[k]=words.len() as u32;
            if let Some(media)=&s.media {
                let time=request.exact()?;
            let base=s.definition.time.as_ref().map(|t|vork_time::exact::Germ::point(t.exact())).unwrap_or_else(||time.clone());
            let clock=self.animation.image_time_exact(&samples,&time,&s.name,base)?;
                words.extend(encode(&s.definition,media.lock().unwrap().frame_exact(&clock,self.fps,self.max_side)?));
            } else {words.extend_from_slice(&s.pixels);}
        }
        Ok(words)
    }
}
fn encode(definition:&v2::Image,image:image::DynamicImage)->Vec<u32> {
    let image=image.into_rgba32f();
    let wrap=|w| match w {v2::Wrap::Repeat=>0,v2::Wrap::ClampToEdge=>1,v2::Wrap::MirroredRepeat=>2};
    let mut words=vec![image.width(),image.height(),wrap(definition.wrap_s),wrap(definition.wrap_t),u32::from(matches!(definition.mag_filter,v2::MagFilter::Nearest))];
    for p in image.pixels() {for (k,&v) in p.0.iter().enumerate() {words.push(if k<3 && definition.is_srgb() {crate::texture_source::decode_srgb(v)} else {v}.to_bits());}}
    words
}
fn sampling_shader()->String {
    format!(r#"
fn environment_address(p:i32,n:i32,mode:u32)->i32 {{
    if mode==1u {{return clamp(p,0,n-1);}}
    if mode==2u {{let q=((p%(2*n))+2*n)%(2*n);return min(q,2*n-1-q);}}
    return ((p%n)+n)%n;
}}
fn environment_image_load(slot:u32,p:vec2<i32>)->vec4<f32> {{
    let size=environment_image_extent(slot);let mode=environment_image_modes(slot);
    let q=vec2(environment_address(p.x,size.x,mode.x),environment_address(p.y,size.y,mode.y));
    return environment_image_texel(slot,q);
}}
fn {sample}(slot:u32,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32> {{
    if slot==4294967295u {{return vec4(1.);}}
    let size=vec2<f32>(environment_image_extent(slot));
    // Bound before integer conversion, including very large authored UVs.
    let mode=environment_image_modes(slot);
    var bounded=uv;
    for(var k=0u;k<2u;k++) {{if mode[k]==1u {{bounded[k]=clamp(uv[k],0.,1.);}} else {{let period=select(1.,2.,mode[k]==2u);bounded[k]=uv[k]-floor(uv[k]/period)*period;}}}}
    let p=bounded*size-.5;let q=vec2<i32>(floor(p));let f=p-floor(p);
    if mode.z!=0u {{return environment_image_load(slot,vec2<i32>(floor(p+.5)));}}
    let a=environment_image_load(slot,q);let b=environment_image_load(slot,q+vec2(1,0));
    let c=environment_image_load(slot,q+vec2(0,1));let d=environment_image_load(slot,q+vec2(1,1));
    return (a+(b-a)*f.x)+((c+(d-c)*f.x)-(a+(b-a)*f.x))*f.y;
}}
"#,sample=vork_slang::SAMPLE_IMAGE)
}
