//! The GPU side of the water displacement: the two ping-pong targets, the
//! three pipelines of the displacement shader module and the encoding of a
//! [`FramePlan`].

use std::num::NonZeroU64;

use super::state::{FramePlan, MAX_SPLATS_PER_FRAME, Pass, WADING_VARS};
use crate::rendering::{
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    shaders::composed::displacement::WATER_DISPLACEMENT,
};

/// Side of the square state textures. `fWadingWaterTextureRes` is 512 but the
/// engine does not read it; 512 follows from the translated shaders' texel constants
/// (half a texel is 1/1024).
pub const TEXTURE_RESOLUTION: u32 = 512;

/// The 0.5 bias says a UNORM colour target, which clamps every store to
/// `[0, 1]`. 16-bit UNORM is the nearest format that behaves: 8-bit UNORM cannot store 0.5 exactly and
/// 16-bit float has a coarser step above 0.5 than below it, and both turn
/// their rounding error into a drift the 0.99 damping cannot absorb (measured
/// on the GPU: a splat that grows into a saturated field instead of decaying).
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Unorm;

/// What 16-bit UNORM needs as a render target: the format itself and the
/// adapter-specific usage flags (the WebGPU baseline allows no render
/// attachment for it). Required of the device: D3D12 and Metal always have
/// them and Vulkan reports them wherever the format supports storage images.
pub const REQUIRED_FORMAT_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
    .union(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);

/// The neutral state every channel is cleared to.
const NEUTRAL: f64 = 0.5;

const SLOT_ADVECT: usize = 0;
const SLOT_STEP: usize = 1;
const SLOT_FIRST_SPLAT: usize = 2;
const SLOT_COUNT: usize = SLOT_FIRST_SPLAT + MAX_SPLATS_PER_FRAME;
/// Dynamic-offset stride of one pass's constants (the WebGPU minimum uniform
/// offset alignment).
const CONSTANTS_STRIDE: u64 = 256;

/// The register file the displacement shaders read as `constants`, padded to
/// the dynamic-offset stride. Which rows a pass reads: the advect pass
/// `REGISTER_TEXTURE_OFFSET` (zw), a splat `REGISTER_ROW_X` and
/// `REGISTER_ROW_Y` (xyz), the step `REGISTER_WADING_VARS` (xyz).
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct PassConstants {
    registers: [[f32; 4]; REGISTER_COUNT],
    padding: [[f32; 4]; PADDING_ROWS],
}

const REGISTER_COUNT: usize = 3;
const PADDING_ROWS: usize = CONSTANTS_STRIDE as usize / 16 - REGISTER_COUNT;
const REGISTER_TEXTURE_OFFSET: usize = 0;
const REGISTER_ROW_X: usize = 1;
const REGISTER_ROW_Y: usize = 2;
const REGISTER_WADING_VARS: usize = 2;

const _: () = assert!(std::mem::size_of::<PassConstants>() as u64 == CONSTANTS_STRIDE);

impl PassConstants {
    fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

/// The constants of every slot of a frame, in slot order.
fn frame_constants(plan: &FramePlan) -> Vec<PassConstants> {
    let mut slots = vec![PassConstants::zeroed(); SLOT_FIRST_SPLAT + plan.splats.len()];
    slots[SLOT_ADVECT].registers[REGISTER_TEXTURE_OFFSET] =
        [0.0, 0.0, plan.texture_offset.x, plan.texture_offset.y];
    slots[SLOT_STEP].registers[REGISTER_WADING_VARS] = WADING_VARS;
    for (slot, splat) in slots[SLOT_FIRST_SPLAT..].iter_mut().zip(&plan.splats) {
        let (row_x, row_y) = splat.rows();
        slot.registers[REGISTER_ROW_X] = [row_x[0], row_x[1], row_x[2], 0.0];
        slot.registers[REGISTER_ROW_Y] = [row_y[0], row_y[1], row_y[2], 0.0];
    }
    slots
}

fn slot_offset(slot: usize) -> u32 {
    (slot as u64 * CONSTANTS_STRIDE) as u32
}

pub struct DisplacementGpu {
    textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    constants: wgpu::Buffer,
    /// Bind group reading target `i`.
    sources: [wgpu::BindGroup; 2],
    advect: wgpu::RenderPipeline,
    splat: wgpu::RenderPipeline,
    step: wgpu::RenderPipeline,
}

impl DisplacementGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let textures = [0, 1].map(|index| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(["water_displacement_a", "water_displacement_b"][index]),
                size: wgpu::Extent3d {
                    width: TEXTURE_RESOLUTION,
                    height: TEXTURE_RESOLUTION,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let views = [0, 1]
            .map(|index| textures[index].create_view(&wgpu::TextureViewDescriptor::default()));
        let constants = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water_displacement_constants"),
            size: SLOT_COUNT as u64 * CONSTANTS_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The step pass samples bilinearly, the other two passes by nearest texel.
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let point = sampler("water_displacement_point", wgpu::FilterMode::Nearest);
        let linear = sampler("water_displacement_linear", wgpu::FilterMode::Linear);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_displacement_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(CONSTANTS_STRIDE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sources = [0, 1].map(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water_displacement_source"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &constants,
                            offset: 0,
                            size: NonZeroU64::new(CONSTANTS_STRIDE),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&point),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&linear),
                    },
                ],
            })
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water_displacement_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = WATER_DISPLACEMENT.compile_runtime(device);
        // The splat blend state is unknown; an opaque overwrite is assumed.
        let pipeline = |label, vertex_entry, fragment_entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment_entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            advect: pipeline(
                "water_displacement_advect",
                "vs_fullscreen",
                "fs_tex_offset",
            ),
            splat: pipeline("water_displacement_splat", "vs_splat", "fs_splat"),
            step: pipeline("water_displacement_step", "vs_step", "fs_step"),
            textures,
            views,
            constants,
            sources,
        }
    }

    /// The state view of target `index`.
    pub fn view(&self, index: usize) -> &wgpu::TextureView {
        &self.views[index]
    }

    pub fn views(&self) -> &[wgpu::TextureView; 2] {
        &self.views
    }

    pub fn texture(&self, index: usize) -> &wgpu::Texture {
        &self.textures[index]
    }

    /// Writes the constants of every pass of the plan.
    pub fn upload(&self, queue: &wgpu::Queue, plan: &FramePlan) {
        if plan.passes.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.constants,
            0,
            bytemuck::cast_slice(&frame_constants(plan)),
        );
    }

    /// Encodes the plan's passes; each is a render pass of its own because
    /// every one reads what the previous wrote. The profiler entry spans them.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        profiler: &GpuProfiler,
        plan: &FramePlan,
    ) {
        for (index, pass) in plan.passes.iter().enumerate() {
            let part = SpanPart::of(index, plan.passes.len());
            let timestamps = profiler.render_writes(GpuPass::WaterDisplacement, part);
            match *pass {
                Pass::Clear { target } => {
                    drop(self.begin(encoder, target, timestamps));
                }
                Pass::Advect {
                    source,
                    destination,
                } => {
                    let mut render = self.begin(encoder, destination, timestamps);
                    render.set_pipeline(&self.advect);
                    render.set_bind_group(0, &self.sources[source], &[slot_offset(SLOT_ADVECT)]);
                    render.draw(0..3, 0..1);
                    render.set_pipeline(&self.splat);
                    for splat in 0..plan.splats.len() {
                        render.set_bind_group(
                            0,
                            &self.sources[source],
                            &[slot_offset(SLOT_FIRST_SPLAT + splat)],
                        );
                        render.draw(0..6, 0..1);
                    }
                }
                Pass::Step {
                    source,
                    destination,
                } => {
                    let mut render = self.begin(encoder, destination, timestamps);
                    render.set_pipeline(&self.step);
                    render.set_bind_group(0, &self.sources[source], &[slot_offset(SLOT_STEP)]);
                    render.draw(0..3, 0..1);
                }
            }
        }
    }

    /// A pass onto `target` that starts from the neutral state.
    fn begin<'encoder>(
        &self,
        encoder: &'encoder mut wgpu::CommandEncoder,
        target: usize,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> wgpu::RenderPass<'encoder> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Displacement Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.views[target],
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: NEUTRAL,
                        g: NEUTRAL,
                        b: NEUTRAL,
                        a: NEUTRAL,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exactly representable results"
)]
mod tests {
    use glam::Vec2;

    use super::*;
    use crate::rendering::water::displacement::state::Splat;

    #[test]
    fn the_constants_of_a_plan_land_in_their_slots() {
        let plan = FramePlan {
            texture_offset: Vec2::new(0.25, -0.5),
            splats: vec![Splat {
                current: Vec2::new(1.0, 0.0),
                previous: Vec2::ZERO,
            }],
            ..FramePlan::default()
        };
        let slots = frame_constants(&plan);
        assert_eq!(slots.len(), SLOT_FIRST_SPLAT + 1);
        assert_eq!(
            slots[SLOT_ADVECT].registers[REGISTER_TEXTURE_OFFSET],
            [0.0, 0.0, 0.25, -0.5]
        );
        assert_eq!(
            slots[SLOT_STEP].registers[REGISTER_WADING_VARS],
            WADING_VARS
        );
        let splat = &slots[SLOT_FIRST_SPLAT].registers;
        assert_eq!(splat[REGISTER_ROW_X], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(splat[REGISTER_ROW_Y], [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn slot_offsets_stay_within_the_buffer() {
        assert_eq!(
            u64::from(slot_offset(SLOT_COUNT - 1)) + CONSTANTS_STRIDE,
            SLOT_COUNT as u64 * CONSTANTS_STRIDE
        );
    }
}
