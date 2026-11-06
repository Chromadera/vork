//! Pack native TLAS descriptors from the exact object snapshot consumed by shading.
//! Native references/options are immutable templates supplied by the backend; the
//! GPU only replaces affine coefficients and visibility. No address arithmetic.
use crate::*;
use std::fmt::Write as _;

pub(crate) struct Placement {
    pub buffer: wgpu::Buffer,
    pub references: Vec<wgpu::Blas>,
    pipeline: wgpu::ComputePipeline,
    group: wgpu::BindGroup,
    count: u32,
    width: u32,
}

/// Generate from backend layout facts, shared by production and packing tests.
fn source(
    stride: u32,
    transform: [u32; 12],
    mask_word: u32,
    mask_shift: u32,
    count: u32,
    width: u32,
) -> String {
    let mut code = format!(
        r#"
{finite_checks}
{object_abi}
@group(0) @binding(0) var<storage,read> objects:array<Object>;
@group(0) @binding(1) var<storage,read> mapping:array<vec4<u32>>;
@group(0) @binding(2) var<storage,read_write> descriptors:array<u32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) gid:vec3<u32>) {{
    let i=gid.x+gid.y*{width}u; if(i>={count}u) {{return;}}
    let object=objects[mapping[i].x];
    var rows=object.model;
    let a=rows[0].xyz; let b=rows[1].xyz; let c=rows[2].xyz;
    let determinant=dot(a,cross(b,c));
    var mask=select(0u,object.metadata.y,mapping[i].z!=0u);
    // A masked descriptor must still carry a finite nonsingular placeholder.
    if(mask==0u || determinant==0. || !finite_f32(determinant) ||
       !finite4(rows[0]) ||
       !finite4(rows[1]) ||
       !finite4(rows[2])) {{
        mask=0u; rows=mat3x4<f32>(vec4(1.,0.,0.,0.),vec4(0.,1.,0.,0.),vec4(0.,0.,1.,0.));
    }}
    if(mapping[i].y!=0u && (mask & {diffuse}u)!=0u) {{ mask=mask | {occluder}u; }}
    if(mapping[i].w!=0u && (mask & {shadow}u)!=0u) {{mask=mask | {solid}u;}}
    let base=i*{words}u;
"#,
        object_abi = crate::object_abi::wgsl("Object"),
        finite_checks = crate::compute::FINITE_WGSL,
        shadow=constants::RAY_SHADOW,
        solid=constants::RAY_SHADOW_SOLID,
        diffuse = constants::RAY_DIFFUSE,
        occluder = constants::RAY_GI_OCCLUDER,
        words = stride / 4
    );
    for (coefficient, offset) in transform.into_iter().enumerate() {
        writeln!(
            code,
            "descriptors[base+{offset}u]=bitcast<u32>(rows[{}][{}]);",
            coefficient / 4,
            coefficient % 4
        )
        .unwrap();
    }
    let bits = 0xffu32 << mask_shift;
    writeln!(code,"descriptors[base+{mask_word}u]=(descriptors[base+{mask_word}u] & {}u) | ((mask & 255u)<<{mask_shift}u);\n}}",!bits).unwrap();
    code
}

impl Placement {
    pub(crate) fn new(
        r: &Renderer,
        objects: &wgpu::Buffer,
        instances: &[(u32, bool, bool, bool, wgpu::Blas)],
        retained: &std::sync::OnceLock<wgpu::ComputePipeline>,
    ) -> Result<Self> {
        let mut bytes = Vec::new();
        let mut mapping = Vec::new();
        let mut references = Vec::new();
        let mut abi = None;
        let mut templates = std::collections::HashMap::new();
        for (object, opaque, hardware_visible, shadow_solid, blas) in instances {
            let (layout, template) = match templates.entry(blas.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    references.push(blas.clone());
                    entry.insert(
                        blas.native_tlas_instance(0, 0)
                            .context("native TLAS reference template")?,
                    )
                }
            };
            if let Some(previous) = abi {
                assert_eq!(previous, *layout);
            } else {
                abi = Some(*layout);
            }
            let custom_index = bytes.len() + layout.custom_index as usize * 4;
            bytes.extend_from_slice(template);
            // The backend owns this field's location. Its template has mask zero;
            // the installed object index is already admitted to the portable 24 bits.
            bytes[custom_index..custom_index + 4].copy_from_slice(&object.to_ne_bytes());
            // Software field bounds keep an identity but never become triangle hits.
            mapping.push([*object, u32::from(*opaque), u32::from(*hardware_visible), u32::from(*shadow_solid)]);
        }
        let layout = abi.context("empty native TLAS placement")?;
        let buffer = init(
            &r.device,
            "native TLAS descriptors",
            &bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::TLAS_INPUT,
        );
        let mapping = init(
            &r.device,
            "TLAS object mapping",
            bytemuck::cast_slice(&mapping),
            wgpu::BufferUsages::STORAGE,
        );
        let width = (instances.len() as u32)
            .div_ceil(64)
            .min(r.device.limits().max_compute_workgroups_per_dimension);
        let pipeline = retained.get_or_init(|| crate::compute::pipeline_layout(
            r,
            &source(
                layout.stride,
                layout.transform,
                layout.mask,
                layout.mask_shift,
                instances.len() as u32,
                width * 64,
            ),
            "native TLAS placement",
            &[Some(true), Some(true), Some(false)],
            "main",
        )).clone();
        let group = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("native TLAS placement"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, objects.as_entire_binding()),
                entry(1, mapping.as_entire_binding()),
                entry(2, buffer.as_entire_binding()),
            ],
        });
        Ok(Self {
            buffer,
            references,
            pipeline,
            group,
            count: instances.len() as u32,
            width,
        })
    }
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, tlas: &wgpu::Tlas) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("native TLAS placement"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(self.width, self.count.div_ceil(64).div_ceil(self.width), 1);
        }
        let references: Vec<_> = self.references.iter().collect();
        // Templates retain their backend-owned native reference and object ID;
        // only the backend-declared transform/mask words were written above.
        unsafe {
            encoder.build_tlas_from_buffer(tlas, &self.buffer, 0, self.count, &references);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_descriptor_packing_preserves_references_and_masks_tails() -> Result<()> {
        let Some(r) = crate::test_renderer()? else {
            return Ok(());
        };
        let mut visible = Object::new(Mat4::from_scale_rotation_translation(
            Vec3::new(-2., 3., 0.5),
            glam::Quat::from_rotation_y(0.7),
            Vec3::new(11., -7., 4.),
        ));
        visible.metadata[1] = constants::RAY_DIFFUSE | constants::RAY_CAMERA;
        let mut tail = visible;
        tail.metadata[1] = 0;
        let collapsed = Object::new(Mat4::from_scale(Vec3::ZERO));
        let objects = init(
            &r.device,
            "packing objects",
            bytemuck::cast_slice(&gpu_objects(&[visible, tail, collapsed])),
            wgpu::BufferUsages::STORAGE,
        );
        let mapping = init(
            &r.device,
            "packing map",
            bytemuck::cast_slice(&[[0u32, 1, 1, 0], [1, 1, 1, 0], [2, 1, 1, 0], [0, 1, 0, 0]]),
            wgpu::BufferUsages::STORAGE,
        );
        // Exercise both native ABIs even on a device without ray-query support.
        for (stride, transform, mask, shift) in [
            (64, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], 12, 24),
            (72, [0, 3, 6, 9, 1, 4, 7, 10, 2, 5, 8, 11], 13, 0),
        ] {
            let words = stride / 4;
            let initial: Vec<u32> = (0..4 * words).map(|i| 0x12345600 + i).collect();
            let output = init(
                &r.device,
                "packing descriptors",
                bytemuck::cast_slice(&initial),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let pipeline = crate::compute::pipeline_layout(
                &r,
                &source(stride, transform, mask, shift, 4, 64),
                "test native packing",
                &[Some(true), Some(true), Some(false)],
                "main",
            );
            let group = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    entry(0, objects.as_entire_binding()),
                    entry(1, mapping.as_entire_binding()),
                    entry(2, output.as_entire_binding()),
                ],
            });
            let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: output.size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = r.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            r.queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |result| {
                tx.send(result).unwrap();
            });
            r.wait(None)?;
            rx.recv()??;
            let bytes = readback.get_mapped_range(..);
            let actual: &[u32] = bytemuck::cast_slice(&bytes);
            let mut expected = initial;
            for i in 0..4usize {
                let rows = if i == 0 {
                    visible.gpu().model
                } else {
                    Object::new(Mat4::IDENTITY).gpu().model
                };
                for (coefficient, offset) in transform.into_iter().enumerate() {
                    expected[i * words as usize + offset as usize] =
                        rows[coefficient / 4][coefficient % 4].to_bits();
                }
                let index = i * words as usize + mask as usize;
                let visibility = if i == 0 {
                    visible.metadata[1] | constants::RAY_GI_OCCLUDER
                } else {
                    0
                };
                expected[index] = (expected[index] & !(255 << shift)) | (visibility << shift);
            }
            assert_eq!(
                actual, expected,
                "native transform order, tail masks and immutable reference/options words"
            );
            drop(bytes);
            readback.unmap();
        }
        r.errors.check()?;
        Ok(())
    }
}
