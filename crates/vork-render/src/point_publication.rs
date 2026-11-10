//! Raster views of compact point publications. The source point/shape domains
//! remain distinct; virtual vertex indices preserve authored primitive order.
use crate::*;

pub(crate) struct Frame {
    pub buffer: wgpu::Buffer,
    copies: Vec<(wgpu::Buffer,u64,u64,u64)>,
}
impl Frame {
    pub(crate) fn new(r:&Renderer,s:&InstalledScene,geometry:&[Option<EvaluatedGeometry>])->Result<Self> {
        let active=geometry.iter().any(|g|g.as_ref().is_some_and(|g|g.point_domain.is_some()));
        if !active {return Ok(Self {buffer:init(&r.device,"no compact points",&[0;4],wgpu::BufferUsages::STORAGE),copies:Vec::new()});}
        let mut words=vec![0u32;1+s.objects.len()];words[0]=s.objects.len().try_into()?;
        let mut initial=Vec::new();let mut copies=Vec::new();
        for (draw,geometry) in s.draws.iter().zip(geometry) {
            let Some(geometry)=geometry else {continue};
            let Some((capacity,_))=geometry.point_domain else {continue};
            let attributes=geometry.attributes.as_ref().context("compact point attributes")?;
            let header=u32::try_from(words.len())?;words.extend([0;4]);
            let position=u32::try_from(words.len())?;
            words.resize(words.len()+geometry.output.size as usize/4,0);
            initial.push((geometry.output.buffer.clone(),geometry.output.offset,u64::from(position)*4,geometry.output.size));
            copies.push((geometry.output.buffer.clone(),geometry.output.offset+8,u64::from(position)*4+8,u64::from(capacity)*40));
            let attribute=u32::try_from(words.len())?;
            words.resize(words.len()+attributes.size() as usize/4,0);
            copies.push((attributes.clone(),0,u64::from(attribute)*4,attributes.size()));
            let history=u32::try_from(words.len())?;
            words.resize(words.len()+geometry.previous_output.size as usize/4,0);
            copies.push((geometry.previous_output.buffer.clone(),geometry.previous_output.offset,u64::from(history)*4,geometry.previous_output.size));
            words[header as usize..header as usize+4].copy_from_slice(&[position,attribute,history,(draw.layout.attribute_stride()/4) as u32]);
            for object in draw.instances.clone() {words[object as usize+1]=header;}
        }
        gpu_memory::storage_binding(&r.device.limits(),"compact point raster publication",words.len() as u64*4)?;
        let buffer=init(&r.device,"compact point raster publication",bytemuck::cast_slice(&words),wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);
        let mut encoder=r.device.create_command_encoder(&Default::default());
        for (source,offset,destination,size) in initial {encoder.copy_buffer_to_buffer(&source,offset,&buffer,destination,size);}
        crate::command_finish::Segments::submit(&r.queue,encoder);
        Ok(Self {buffer,copies})
    }
    pub(crate) fn encode(&self,encoder:&mut wgpu::CommandEncoder) {
        for (source,offset,destination,size) in &self.copies {encoder.copy_buffer_to_buffer(source,*offset,&self.buffer,*destination,*size);}
    }
}
