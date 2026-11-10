//! Ordered geometry execution. Every lowering consumes and returns the same
//! draw-indexed geometry packet; numerical kernels remain in their owners.
use crate::*;
use vork_scene::v2;

/// A packet retains immutable topology, domains, source correspondence and
/// object/instance identity through its installed draw. Only owned evaluated
/// columns change. The sample request is shared by the whole frame generation.
struct Geometry {
    draw: usize,
    evaluated: Option<EvaluatedGeometry>,
}
impl Geometry {
    fn publish(&mut self, s: &InstalledScene, mut output: EvaluatedGeometry) -> Result<()> {
        let draw = &s.draws[self.draw];
        let bytes = u64::from(draw.vertex_count) * std::mem::size_of::<DeformedVertex>() as u64;
        let capacity_matches=if let Some((capacity,shape_vertices))=output.point_domain {
            u64::from(capacity)*u64::from(shape_vertices)==u64::from(draw.vertex_count)
                && output.output.size==(2+u64::from(capacity)*10+u64::from(shape_vertices)*5)*4
                && output.previous_output.size==u64::from(capacity)*80
        } else {output.output.size==bytes && output.previous_output.size==bytes*2};
        ensure!(
            capacity_matches,
            "geometry stage changed vertex capacity without a topology transition for draw {}",
            self.draw
        );
        if output.attributes.is_none() {
            output.attributes = self
                .evaluated
                .as_ref()
                .and_then(|input| input.attributes.clone());
            if output.previous_attributes.is_none() {
                output.previous_attributes=self.evaluated.as_ref().and_then(|input|input.previous_attributes.clone());
            }
        }
        if let Some(attributes) = &output.attributes {
            ensure!(
                attributes.size() >= u64::from(output.point_domain.map_or(draw.vertex_count,|p|p.0)) * draw.layout.attribute_stride(),
                "geometry stage lost carried attribute rows for draw {}",
                self.draw
            );
        }
        if let Some(attributes)=&output.previous_attributes {
            ensure!(attributes.size()>=u64::from(output.point_domain.map_or(draw.vertex_count,|p|p.0))*draw.layout.attribute_stride()*2,
                "geometry stage lost historical attribute rows for draw {}",self.draw);
        }
        self.evaluated = Some(output);
        Ok(())
    }
}

/// A lowering installs resources against its predecessor packets. The ordered
/// list, rather than the executor, chooses which implementations to install.
type Lowering = fn(&Renderer, &InstalledScene, &mut [Geometry]) -> Result<Box<dyn Stage>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind { Deform, Compute, Displace }
impl Kind {
    pub(crate) const DEFAULT: [Self;3] = [Self::from_scene(v2::GeometryStage::DEFAULT[0]),Self::from_scene(v2::GeometryStage::DEFAULT[1]),Self::from_scene(v2::GeometryStage::DEFAULT[2])];
    pub(crate) const fn from_scene(stage:v2::GeometryStage)->Self {match stage {v2::GeometryStage::Deform=>Self::Deform,v2::GeometryStage::Compute=>Self::Compute,v2::GeometryStage::Displace=>Self::Displace}}
    pub(crate) fn deformation_before_compute(list: &[Self]) -> bool {
        Self::before(list,Self::Deform,Self::Compute)
    }
    pub(crate) fn before(list:&[Self],a:Self,b:Self) -> bool {
        list.iter().position(|kind|*kind==a).zip(list.iter().position(|kind|*kind==b))
            .is_some_and(|(deform,compute)|deform<compute)
    }
    fn lowering(self, s: &InstalledScene) -> Option<Lowering> {
        match self {
            Self::Deform => s.draws.iter().any(|draw|draw.deform.is_some()).then_some(Deform::install as Lowering),
            Self::Compute => s.draws.iter().any(|draw|draw.computed).then_some(Compute::install as Lowering),
            Self::Displace => s.displacement.is_some().then_some(Displace::install as Lowering),
        }
    }
}

/// Every implementation consumes the same evaluated geometry contract. Builtin
/// resource accessors are optional hooks for the existing fused fast path.
trait Stage: Send + Sync {
    fn append_draw_inputs(&self, _:usize, _: &values::ChangeTracker, _: &mut Vec<u8>) {}
    fn skin(&self) -> &[Option<FrameDeform>] {
        &[]
    }
    fn subdivision_morphs(&self) -> &[Option<wgpu::Buffer>] {
        &[]
    }
    fn displacements(&self) -> &[Option<displacement::Frame>] {
        &[]
    }
    fn prepare_history(
        &self,
        _: &Renderer,
        _: &InstalledScene,
        _: &[vork_time::Request],
        _: bool,
        _: &mut Option<compute::Checkpoint>,
    ) -> Result<()> {
        Ok(())
    }
    fn spatial_dirty(&self, _: usize, _: &values::ChangeTracker) -> bool {
        false
    }
    fn append_inputs(&self, _: &values::ChangeTracker, _: &mut Vec<u8>) {}
    /// The legacy graph records the builtin prefix in its deformation pass.
    /// Stages that evaluate before visibility terminate that prefix.
    fn encode_prefix(
        &self,
        _: &mut wgpu::CommandEncoder,
        _: &InstalledScene,
        _: Option<&wgpu::QuerySet>,
    ) -> bool {
        false
    }
    fn evaluate(
        &self,
        _: &[Box<dyn Stage>],
        _: &mut Evaluation<'_>,
    ) -> Result<Vec<Option<(Vec3, Vec3)>>> {
        Ok(Vec::new())
    }
}

struct Evaluation<'a> {
    predecessor_inputs: Option<Vec<Vec<u8>>>,
    r: &'a Renderer,
    s: &'a InstalledScene,
    cached: &'a [Option<(Vec3, Vec3)>],
    changes: &'a mut values::ChangeTracker,
    request: &'a vork_time::Request,
    previous: &'a vork_time::Request,
    checkpoint: &'a mut Option<compute::Checkpoint>,
    materials: Option<&'a materials::FrameMaterials>,
    objects: &'a [Object],
    spaces: &'a [[Mat4; 3]],
    queries: &'a wgpu::QuerySet,
    time: f32,
    previous_time: f32,
    next_time: f32,
}

struct Deform {
    chained: bool,
    skin: Vec<Option<FrameDeform>>,
    subdivision_morphs: Vec<Option<wgpu::Buffer>>,
}
impl Deform {
    fn install(r: &Renderer, s: &InstalledScene, input: &mut [Geometry]) -> Result<Box<dyn Stage>> {
        let predecessors: Vec<_> = input.iter().map(|packet|packet.evaluated.clone()).collect();
        let mut skin = passes::deform::resources(s, r, &predecessors)?;
        let chained = skin.iter().flatten().any(|frame|frame.chained.is_some());
        let subdivision_morphs = subdivision::frame_resources(s, r, &mut skin);
        for (packet, source) in input.iter_mut().zip(&skin) {
            if let Some(source) = source {
                packet.publish(s, EvaluatedGeometry::from(source))?;
            }
        }
        Ok(Box::new(Self {
            chained,
            skin,
            subdivision_morphs,
        }))
    }
}
impl Stage for Deform {
    fn append_draw_inputs(&self,draw:usize,changes:&values::ChangeTracker,inputs:&mut Vec<u8>) {
        if let Some(frame)=self.skin.get(draw).and_then(Option::as_ref) {
            changes.append_buffer_inputs(&frame.params,inputs);changes.append_buffer_inputs(&frame.previous_params,inputs);
        }
        if let Some(morphs)=self.subdivision_morphs.get(draw).and_then(Option::as_ref) {changes.append_buffer_inputs(morphs,inputs);}
    }
    fn skin(&self) -> &[Option<FrameDeform>] {
        &self.skin
    }
    fn subdivision_morphs(&self) -> &[Option<wgpu::Buffer>] {
        &self.subdivision_morphs
    }
    fn spatial_dirty(&self, draw: usize, changes: &values::ChangeTracker) -> bool {
        self.skin
            .get(draw)
            .and_then(Option::as_ref)
            .is_some_and(|frame| {
                changes.buffer_dirty(&frame.params) || changes.buffer_dirty(&frame.previous_params)
            })
            || self
                .subdivision_morphs
                .get(draw)
                .and_then(Option::as_ref)
                .is_some_and(|buffer| changes.buffer_dirty(buffer))
    }
    fn append_inputs(&self, changes: &values::ChangeTracker, inputs: &mut Vec<u8>) {
        for frame in self.skin.iter().flatten() {
            changes.append_buffer_inputs(&frame.params, inputs);
            changes.append_buffer_inputs(&frame.previous_params, inputs);
        }
        for buffer in self.subdivision_morphs.iter().flatten() {
            changes.append_buffer_inputs(buffer, inputs);
        }
    }
    fn encode_prefix(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        s: &InstalledScene,
        queries: Option<&wgpu::QuerySet>,
    ) -> bool {
        if self.chained { return false; }
        passes::deform::dispatch(encoder, s, &self.skin, queries);
        true
    }
    fn evaluate(&self, _: &[Box<dyn Stage>], e: &mut Evaluation<'_>) -> Result<Vec<Option<(Vec3,Vec3)>>> {
        if !self.chained { return Ok(Vec::new()); }
        let mut encoder=e.r.device.create_command_encoder(&Default::default());
        if !e.changes.bytes.is_empty() {
            let upload=init(&e.r.device,"geometry stage pose uploads",&e.changes.bytes,wgpu::BufferUsages::COPY_SRC);
            for (destination,offset,source,size) in &e.changes.uploads {
                encoder.copy_buffer_to_buffer(&upload,*source,destination,*offset,*size);
            }
        }
        passes::deform::dispatch_chained(&mut encoder,e.s,&self.skin);
        let jobs:Vec<_>=self.skin.iter().enumerate().filter_map(|(index,frame)|frame.as_ref()
            .filter(|frame|frame.leader==index).and_then(|frame|frame.chained.as_ref()).map(|chained|(index,chained))).collect();
        for (_,job) in &jobs {job.bounds.encode(&mut encoder);}
        let submission=crate::command_finish::Segments::submit(&e.r.queue,encoder);
        e.r.errors.check()?;
        let (tx,rx)=std::sync::mpsc::channel();
        for (_,job) in &jobs {let tx=tx.clone();job.bounds.map(move |result|{let _=tx.send(result);});}
        drop(tx);e.r.wait(Some(submission))?;
        for result in rx {result?;}
        let mut output=vec![None;e.s.draws.len()];
        for (index,job) in jobs {output[index]=Some(job.bounds.collect()?);}
        for (index,frame) in self.skin.iter().enumerate().filter_map(|(i,frame)|frame.as_ref().map(|frame|(i,frame))) {
            if frame.chained.is_some() {output[index]=output[frame.leader];}
        }
        Ok(output)
    }
}

struct Compute {
    carried: Vec<(usize,crate::geometry_stage_source::Carried)>,
    frames: Vec<Option<computed_geometry::Frame>>,
    sources: Vec<String>,
    draw_sources: Option<Vec<Vec<String>>>,
}
impl Compute {
    fn install(r: &Renderer, s: &InstalledScene, input: &mut [Geometry]) -> Result<Box<dyn Stage>> {
        let frames = computed_geometry::resources(r, s)?;
        let mut carried=Vec::new();
        for (index,(packet, source)) in input.iter_mut().zip(&frames).enumerate() {
            if let Some(source) = source {
                let mut output=source.geometry.clone();
                if let Some(predecessor)=&packet.evaluated {
                    let draw=&s.draws[index];
                    let NodeKind::Mesh {geometry,..}=&s.nodes[s.roots[draw.object as usize].node].node.kind else {unreachable!()};
                    let shape=&s.scene.geometry[geometry];
                    ensure!(matches!(shape.topology,vork_scene::v2::Topology::Triangles),
                        "node {:?}: compute after native point deformation needs point/corner correspondence; compute point deformation in the producer, put deformation after compute, or use triangles",s.nodes[s.roots[draw.object as usize].node].node.name);
                    let owns=|semantic:&str|shape.attributes.get(semantic).is_some_and(|name|crate::compute::writes_buffer(&s.scene,name));
                    let position=owns("POSITION");
                    let mask=[!position,!owns("NORMAL") && (!position || shape.attributes.contains_key("NORMAL")),
                        !owns("TANGENT") && (!position || shape.attributes.contains_key("TANGENT"))];
                    if mask[0] {output.gpu_bounds=None;}
                    if mask.iter().any(|&carry|carry) {
                        carried.push((index,crate::geometry_stage_source::Carried::new(r,predecessor,&source.geometry,mask)?));
                    }
                }
                packet.publish(s, output)?;
            }
        }
        let sources=s.computed_geometry.iter().flatten().flat_map(|mesh|mesh.source_names().iter().cloned())
            .collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let draw_sources=general_execution(s).then(||s.computed_geometry.iter().map(|mesh|mesh.as_ref()
            .map_or_else(Vec::new,|mesh|mesh.source_names().to_vec())).collect());
        Ok(Box::new(Self {carried,frames,sources,draw_sources}))
    }
}
impl Stage for Compute {
    fn append_draw_inputs(&self,draw:usize,changes:&values::ChangeTracker,inputs:&mut Vec<u8>) {
        if let Some(sources)=self.draw_sources.as_ref().and_then(|sources|sources.get(draw)) {
            for source in sources {inputs.extend(changes.stream_version(source).to_le_bytes());}
        }
    }
    fn append_inputs(&self, changes:&values::ChangeTracker, inputs:&mut Vec<u8>) {
        for source in &self.sources {inputs.extend(changes.stream_version(source).to_le_bytes());}
    }
    fn prepare_history(
        &self,
        r: &Renderer,
        s: &InstalledScene,
        samples: &[vork_time::Request],
        cut: bool,
        checkpoint: &mut Option<compute::Checkpoint>,
    ) -> Result<()> {
        let times = samples
            .iter()
            .map(|request| {
                if cut {
                    Ok(request.clone())
                } else {
                    request.previous(s.render.fps)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        computed_geometry::prepare_history(r, s, &self.frames, &times, checkpoint)
    }
    fn evaluate(
        &self,
        predecessors: &[Box<dyn Stage>],
        e: &mut Evaluation<'_>,
    ) -> Result<Vec<Option<(Vec3, Vec3)>>> {
        let mut output=computed_geometry::evaluate(
            e.r,
            e.s,
            &self.frames,
            e.cached,
            e.changes,
            e.request,
            e.previous,
            e.checkpoint,
        )?;
        if self.carried.is_empty() {return Ok(output);}
        let mut encoder=e.r.device.create_command_encoder(&Default::default());
        if !e.changes.bytes.is_empty() {
            let upload=init(&e.r.device,"carried geometry pose uploads",&e.changes.bytes,wgpu::BufferUsages::COPY_SRC);
            for (destination,offset,source,size) in &e.changes.uploads {encoder.copy_buffer_to_buffer(&upload,*source,destination,*offset,*size);}
        }
        encode_prefix(predecessors,&mut encoder,e.s,None);
        for (_,job) in &self.carried {job.encode(&mut encoder);}
        let submission=crate::command_finish::Segments::submit(&e.r.queue,encoder);
        e.r.errors.check()?;
        let jobs:Vec<_>=self.carried.iter().filter_map(|(index,job)|job.bounds.as_ref().map(|bounds|(*index,bounds))).collect();
        if !jobs.is_empty() {
            let (tx,rx)=std::sync::mpsc::channel();
            for (_,bounds) in &jobs {let tx=tx.clone();bounds.map(move |result|{let _=tx.send(result);});}
            drop(tx);e.r.wait(Some(submission))?;
            for result in rx {result?;}
            for (index,bounds) in jobs {output[index]=Some(bounds.collect()?);}
        }
        Ok(output)
    }
}

struct Displace(Vec<Option<displacement::Frame>>);
impl Displace {
    fn install(r: &Renderer, s: &InstalledScene, input: &mut [Geometry]) -> Result<Box<dyn Stage>> {
        let predecessors: Vec<_> = input
            .iter()
            .map(|packet| packet.evaluated.clone())
            .collect();
        let frames = displacement::resources(r, s, &predecessors)?;
        for packet in input {
            let leader = s.draws[packet.draw]
                .displacement_leader
                .unwrap_or(packet.draw);
            if let Some(source) = frames.get(leader).and_then(Option::as_ref) {
                packet.publish(s, source.geometry.clone())?;
            }
        }
        Ok(Box::new(Self(frames)))
    }
}
impl Stage for Displace {
    fn append_draw_inputs(&self,draw:usize,changes:&values::ChangeTracker,inputs:&mut Vec<u8>) {
        if let Some(frame)=self.0.get(draw).and_then(Option::as_ref) {frame.append_inputs(changes,inputs);}
    }
    fn displacements(&self) -> &[Option<displacement::Frame>] {
        &self.0
    }
    fn spatial_dirty(&self, draw: usize, changes: &values::ChangeTracker) -> bool {
        self.0
            .get(draw)
            .and_then(Option::as_ref)
            .is_some_and(|frame| frame.dirty(changes))
    }
    fn append_inputs(&self, changes: &values::ChangeTracker, inputs: &mut Vec<u8>) {
        for frame in self.0.iter().flatten() {
            frame.append_inputs(changes, inputs);
        }
    }
    fn evaluate(
        &self,
        predecessors: &[Box<dyn Stage>],
        e: &mut Evaluation<'_>,
    ) -> Result<Vec<Option<(Vec3, Vec3)>>> {
        displacement::evaluate(
            e.r,
            e.s,
            predecessors
                .iter()
                .map(|stage| stage.skin())
                .find(|skin| !skin.is_empty())
                .unwrap_or(&[]),
            &self.0,
            e.materials,
            e.objects,
            e.spaces,
            e.cached,
            e.queries,
            e.changes,
            e.time,
            e.previous_time,
            e.next_time,
            e.predecessor_inputs.as_deref(),
            |encoder| encode_prefix(predecessors, encoder, e.s, None),
        )
    }
}

pub(crate) struct Frame {
    general: bool,
    stages: Vec<Box<dyn Stage>>,
    geometry: Vec<Geometry>,
}
impl Frame {
    pub(crate) fn new(r: &Renderer, s: &InstalledScene) -> Result<Self> {
        // Existing scenes retain their exact fused default. An empty list has
        // no resources, dispatches or implicit input work.
        Self::install(r, s, s.geometry_stages.iter().filter_map(|kind|kind.lowering(s)))
    }
    fn install(
        r: &Renderer,
        s: &InstalledScene,
        list: impl IntoIterator<Item = Lowering>,
    ) -> Result<Self> {
        let mut geometry: Vec<_> = (0..s.draws.len())
            .map(|draw| Geometry {
                draw,
                evaluated: None,
            })
            .collect();
        let mut stages = Vec::new();
        for lower in list {
            stages.push(lower(r, s, &mut geometry)?);
        }
        Ok(Self { general:general_execution(s),stages, geometry })
    }
    pub(crate) fn outputs(&self) -> Vec<Option<EvaluatedGeometry>> {
        self.geometry
            .iter()
            .map(|packet| packet.evaluated.clone())
            .collect()
    }
    pub(crate) fn skin(&self) -> &[Option<FrameDeform>] {
        self.stages
            .iter()
            .map(|stage| stage.skin())
            .find(|skin| !skin.is_empty())
            .unwrap_or(&[])
    }
    pub(crate) fn subdivision_morphs(&self) -> &[Option<wgpu::Buffer>] {
        self.stages
            .iter()
            .map(|stage| stage.subdivision_morphs())
            .find(|morphs| !morphs.is_empty())
            .unwrap_or(&[])
    }
    pub(crate) fn displacements(&self) -> &[Option<displacement::Frame>] {
        self.stages
            .iter()
            .map(|stage| stage.displacements())
            .find(|frames| !frames.is_empty())
            .unwrap_or(&[])
    }
    /// Common sampled-pose upload for the fused builtin stage. The animation
    /// owner supplies its shared palette; geometry owns refinement and streams.
    pub(crate) fn upload_pose(
        &self,
        s: &InstalledScene,
        draw_id: usize,
        pose: usize,
        matrices: &[Mat4],
        weights: &[f32],
        changes: &mut values::ChangeTracker,
    ) -> Result<Option<(Vec3, Vec3)>> {
        let deform = s.draws[draw_id]
            .deform
            .as_ref()
            .context("pose upload needs a deformation stage")?;
        let Some(frame) = self.skin().get(draw_id).and_then(Option::as_ref) else {return Ok(None);};
        let current = pose == 0;
        let mut bounds_out = None;
        let cpu_palette = if let Some(cpu) = &deform.cpu {
            let mut input = Vec::new();
            input.extend_from_slice(bytemuck::cast_slice(matrices));
            input.extend_from_slice(bytemuck::cast_slice(weights));
            let base_vertices = &cpu.base;
            let bounds = changes.derive_upload(
                values::ChangeKey::Subdivision(deform.job, pose),
                &input,
                self.subdivision_morphs()[draw_id].as_ref().unwrap(),
                pose as u64 * deform.count as u64 * constants::MORPH_FLOATS as u64 * 4,
                || {
                    let vertices = cpu.vertices(matrices, weights);
                    ensure!(
                        vertices
                            .iter()
                            .flat_map(|v| v.POSITION)
                            .all(|v| v.is_finite()),
                        "deformed subdivision produced nonfinite positions"
                    );
                    let mut delta =
                        Vec::<f32>::with_capacity(vertices.len() * constants::MORPH_FLOATS);
                    let mut lo = Vec3::splat(f32::INFINITY);
                    let mut hi = Vec3::splat(f32::NEG_INFINITY);
                    for (v, base) in vertices.into_iter().zip(base_vertices) {
                        let p = Vec3::from(v.POSITION);
                        lo = lo.min(p);
                        hi = hi.max(p);
                        delta.extend((p - Vec3::from(base.POSITION)).to_array());
                        delta.extend((Vec3::from(v.NORMAL) - Vec3::from(base.NORMAL)).to_array());
                        delta.extend(
                            (Vec3::from_slice(&v.TANGENT[..3])
                                - Vec3::from_slice(&base.TANGENT[..3]))
                            .to_array(),
                        );
                    }
                    Ok((
                        [lo.to_array(), hi.to_array()],
                        bytemuck::cast_slice(&delta).to_vec(),
                    ))
                },
            )?;
            bounds_out = Some((Vec3::from(bounds[0]), Vec3::from(bounds[1])));
            Some((
                vec![Mat4::IDENTITY],
                (0..3)
                    .map(|i| if i == pose { 1. } else { 0. })
                    .collect::<Vec<_>>(),
            ))
        } else {
            if current {
                bounds_out = Some(values::deformed_bounds(&deform.bounds, matrices, weights));
            }
            None
        };
        let (matrices, weights): (&[Mat4], &[f32]) = cpu_palette
            .as_ref()
            .map_or((matrices, weights), |(m, w)| (m, w));
        let mut params = Vec::with_capacity(4 + matrices.len() * 16 + deform.targets);
        params.extend(crate::passes::deform::header(deform));
        for matrix in matrices {
            params.extend(matrix.to_cols_array());
        }
        params.extend((0..deform.targets).map(|index| weights.get(index).copied().unwrap_or(0.)));
        let blocks = std::iter::once(0..4)
            .chain((0..deform.joints).map(|joint| 4 + joint * 16..4 + (joint + 1) * 16))
            .chain((4 + deform.joints * 16..params.len()).map(|index| index..index + 1));
        for block in blocks {
            changes.upload(
                if current {
                    &frame.params
                } else {
                    &frame.previous_params
                },
                (block.start * std::mem::size_of::<f32>()) as u64
                    + if pose == 2 {
                        frame.previous_params.size() / 2
                    } else {
                        0
                    },
                bytemuck::cast_slice(&params[block]),
            );
        }
        Ok(bounds_out)
    }
    pub(crate) fn prepare_history(
        &self,
        r: &Renderer,
        s: &InstalledScene,
        samples: &[vork_time::Request],
        cut: bool,
        checkpoint: &mut Option<compute::Checkpoint>,
    ) -> Result<()> {
        for stage in &self.stages {
            stage.prepare_history(r, s, samples, cut, checkpoint)?;
        }
        Ok(())
    }
    pub(crate) fn spatial_dirty(&self, draw: usize, changes: &values::ChangeTracker) -> bool {
        changes.dirty(values::ChangeKey::ComputedPositions(draw))
            || self
                .stages
                .iter()
                .any(|stage| stage.spatial_dirty(draw, changes))
    }
    pub(crate) fn append_inputs(&self, changes: &values::ChangeTracker, inputs: &mut Vec<u8>) {
        for stage in &self.stages {
            stage.append_inputs(changes, inputs);
        }
    }
    pub(crate) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        s: &InstalledScene,
        queries: &wgpu::QuerySet,
    ) {
        encode_prefix(&self.stages, encoder, s, Some(queries));
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn evaluate(
        &self,
        r: &Renderer,
        s: &InstalledScene,
        cached: &[Option<(Vec3, Vec3)>],
        changes: &mut values::ChangeTracker,
        request: &vork_time::Request,
        previous: &vork_time::Request,
        checkpoint: &mut Option<compute::Checkpoint>,
        materials: Option<&materials::FrameMaterials>,
        objects: &[Object],
        spaces: &[[Mat4; 3]],
        queries: &wgpu::QuerySet,
        time: f32,
        previous_time: f32,
        next_time: f32,
    ) -> Result<Vec<Option<(Vec3, Vec3)>>> {
        let mut bounds = vec![None; s.draws.len()];
        let mut evaluation = Evaluation {
            predecessor_inputs:self.general.then(||vec![Vec::new();s.draws.len()]),
            r,
            s,
            cached,
            changes,
            request,
            previous,
            checkpoint,
            materials,
            objects,
            spaces,
            queries,
            time,
            previous_time,
            next_time,
        };
        for (index, stage) in self.stages.iter().enumerate() {
            let next = stage.evaluate(&self.stages[..index], &mut evaluation)?;
            if let Some(inputs)=&mut evaluation.predecessor_inputs {
                for (draw,inputs) in inputs.iter_mut().enumerate() {stage.append_draw_inputs(draw,evaluation.changes,inputs);}
            }
            for (output, next) in bounds.iter_mut().zip(next) {
                if next.is_some() {
                    *output = next;
                }
            }
        }
        // A later GPU publication replaces the predecessor's spatial domain.
        // Unknown to the CPU must not retain an earlier stage's smaller bounds.
        for (bounds,packet) in bounds.iter_mut().zip(&self.geometry) {
            if packet.evaluated.as_ref().is_some_and(|g|g.gpu_bounds.is_some()) {*bounds=None;}
        }
        Ok(bounds)
    }
}

fn general_execution(s:&InstalledScene) -> bool {
    s.geometry_stages.as_slice()!=Kind::DEFAULT.as_slice() || s.draws.iter().any(|draw|draw.computed && draw.displacement.is_some())
}

fn encode_prefix(
    stages: &[Box<dyn Stage>],
    encoder: &mut wgpu::CommandEncoder,
    s: &InstalledScene,
    queries: Option<&wgpu::QuerySet>,
) {
    for stage in stages {
        if !stage.encode_prefix(encoder, s, queries) {
            break;
        }
    }
}
