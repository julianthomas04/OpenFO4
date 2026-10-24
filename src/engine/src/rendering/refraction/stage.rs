//! The refraction stage: draws the resident heat-haze cards in the translucent pass, each
//! pixel replaced by the scene colour copy sampled at a distorted position (see
//! [`super::offset`]).
//!
//! Per frame: [`RefractionStage::prepare`] culls the placements, orders them back to front,
//! evaluates the UV tracks and fills the uniforms; [`RefractionStage::draw`] only binds and
//! issues. A card whose normal maps cannot be loaded is skipped with one warning.

use std::num::NonZeroU64;
use std::sync::Arc;

use glam::Vec3;

use super::offset::{animated_haze, layer_transforms};
use super::uniforms::{DRAW_STRIDE, DrawBlock, FrameBlock};
use crate::assets::types::effect_material::RefractionMaterial;
use crate::rendering::effects::draw_buffer::{DrawBuffer, DrawBufferSpec};
use crate::rendering::effects::geometry::{EffectGeometryPool, EffectMeshData, RefractionInstance};
use crate::rendering::effects::lifetime::{SweepClock, WeakSet};
use crate::rendering::effects::material_bindings::{
    BuiltGroup, ColourSpace, EffectTextureError, GroupCache, TextureCache, effect_texture_path,
};
use crate::rendering::effects::shader::VertexLayoutKind;
use crate::rendering::effects::warmup::{JobWorker, Readiness};
use crate::rendering::frustum::clip_planes;
use crate::rendering::lighting::frame_uniform::FrameUniform;
use crate::rendering::post::hdr_target::HDR_FORMAT;
use crate::rendering::scene_copy::SceneCopy;
use crate::rendering::texture::Texture;
use crate::utils::warn_once;

const SHADER: &str = include_str!("../shaders/refraction.wgsl");
const SCENE_GROUP: u32 = 0;
const FRAME_GROUP: u32 = 1;
const MATERIAL_GROUP: u32 = 2;
const DRAW_GROUP: u32 = 3;
const NORMAL_ANISOTROPY: u16 = 4;
const FLAT_NORMAL_TEXEL: [u8; 4] = [128, 128, 255, 255];
const INITIAL_DRAW_CAPACITY: usize = 16;

/// What one frame's [`RefractionStage::prepare`] needs besides the resident placements.
pub struct RefractionFrameInputs<'a> {
    pub frame: &'a FrameUniform,
    /// Camera near plane, eu.
    pub near_plane: f32,
    /// Engine time: the clock the UV tracks run on.
    pub elapsed_seconds: f32,
}

struct Layouts {
    frame: wgpu::BindGroupLayout,
    material: wgpu::BindGroupLayout,
    draw: wgpu::BindGroupLayout,
    pipeline: wgpu::PipelineLayout,
}

fn uniform_entry(dynamic: bool, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

fn normal_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Layouts {
    fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("refraction_frame_layout"),
            entries: &[uniform_entry(false, size_of::<FrameBlock>() as u64)],
        });
        let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("refraction_material_layout"),
            entries: &[
                normal_texture_entry(0),
                normal_texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let draw = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("refraction_draw_layout"),
            entries: &[uniform_entry(true, size_of::<DrawBlock>() as u64)],
        });
        let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("refraction_pipeline_layout"),
            bind_group_layouts: &[
                Some(scene_layout),
                Some(&frame),
                Some(&material),
                Some(&draw),
            ],
            immediate_size: 0,
        });
        Self {
            frame,
            material,
            draw,
            pipeline,
        }
    }
}

/// The haze pipeline: the distorted colour alpha-blended over the target (the weight is the
/// fragment alpha), no depth write, depth tested against the opaque depth.
fn create_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    double_sided: bool,
) -> wgpu::RenderPipeline {
    let keep_destination_alpha = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if double_sided {
            "refraction_pipeline_double_sided"
        } else {
            "refraction_pipeline"
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(VertexLayoutKind::Mesh.buffer_layout())],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: keep_destination_alpha,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: (!double_sided).then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds a material's normal-map bind group; shared with the warm-up worker thread.
struct NormalMapLoader {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    textures: TextureCache,
    flat: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

/// What the worker answers for a material it was asked to build.
type MaterialWarmup = (
    Arc<RefractionMaterial>,
    Result<BuiltGroup, EffectTextureError>,
);

impl NormalMapLoader {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        asset_root: &str,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let flat = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("refraction_flat_normal"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &FLAT_NORMAL_TEXEL,
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("refraction_normal_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: NORMAL_ANISOTROPY,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            layout: layout.clone(),
            textures: TextureCache::new(device, queue, asset_root),
            flat,
            sampler,
        }
    }

    /// The view of a stored texture path; an empty path is the flat normal.
    fn view(&self, stored: &str) -> Result<NormalView, EffectTextureError> {
        if stored.is_empty() {
            return Ok(NormalView::Flat(self.flat.clone()));
        }
        self.textures
            .view(&effect_texture_path(stored), ColourSpace::Linear)
            .map(NormalView::Loaded)
    }

    fn build_group(&self, material: &RefractionMaterial) -> Result<BuiltGroup, EffectTextureError> {
        let views = [
            self.view(&material.normal_a)?,
            self.view(&material.normal_b)?,
        ];
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("refraction_material_bind_group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(views[0].view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(views[1].view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let textures = views
            .into_iter()
            .filter_map(|view| match view {
                NormalView::Loaded(view) => Some(view),
                NormalView::Flat(_) => None,
            })
            .collect();
        Ok(BuiltGroup { group, textures })
    }
}

enum NormalView {
    Flat(wgpu::TextureView),
    Loaded(Arc<wgpu::TextureView>),
}

impl NormalView {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Flat(view) => view,
            Self::Loaded(view) => view,
        }
    }
}

struct PreparedDraw {
    double_sided: bool,
    material: Arc<RefractionMaterial>,
    mesh: Arc<EffectMeshData>,
}

const DRAW_BUFFER: DrawBufferSpec = DrawBufferSpec {
    buffer_label: "refraction_draw_blocks",
    group_label: "refraction_draw_bind_group",
    binding: 0,
    block_size: size_of::<DrawBlock>() as u64,
    stride: DRAW_STRIDE,
};

/// Placements ordered farthest first, so nearer haze samples the same scene copy and wins.
pub fn back_to_front(view_depths: &[f32], order: &mut Vec<usize>) {
    order.clear();
    order.extend(0..view_depths.len());
    order.sort_by(|&a, &b| view_depths[b].total_cmp(&view_depths[a]));
}

pub struct RefractionStage {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layouts: Layouts,
    pipelines: [wgpu::RenderPipeline; 2],
    geometry: EffectGeometryPool,
    loader: Arc<NormalMapLoader>,
    worker: JobWorker<Arc<RefractionMaterial>, MaterialWarmup>,
    groups: GroupCache<RefractionMaterial>,
    pending_materials: WeakSet<RefractionMaterial>,
    failed_materials: WeakSet<RefractionMaterial>,
    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    draw_buffer: DrawBuffer,
    draws: Vec<PreparedDraw>,
    instances: Vec<RefractionInstance>,
    depths: Vec<f32>,
    order: Vec<usize>,
    blocks: Vec<DrawBlock>,
    sweep: SweepClock,
}

impl RefractionStage {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        asset_root: &str,
    ) -> Self {
        let layouts = Layouts::new(device, scene_layout);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("refraction"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("refraction_frame_block"),
            size: size_of::<FrameBlock>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("refraction_frame_bind_group"),
            layout: &layouts.frame,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let loader = Arc::new(NormalMapLoader::new(
            device,
            queue,
            &layouts.material,
            asset_root,
        ));
        let worker = JobWorker::spawn_with("refraction-warmup", {
            let loader = Arc::clone(&loader);
            move |material: Arc<RefractionMaterial>| {
                let built = loader.build_group(&material);
                (material, built)
            }
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines: [false, true].map(|double_sided| {
                create_pipeline(device, &module, &layouts.pipeline, double_sided)
            }),
            draw_buffer: DrawBuffer::new(device, &layouts.draw, DRAW_BUFFER, INITIAL_DRAW_CAPACITY),
            layouts,
            geometry: EffectGeometryPool::default(),
            loader,
            worker,
            groups: GroupCache::default(),
            pending_materials: WeakSet::default(),
            failed_materials: WeakSet::default(),
            frame_buffer,
            frame_group,
            draws: Vec::new(),
            instances: Vec::new(),
            depths: Vec::new(),
            order: Vec::new(),
            blocks: Vec::new(),
            sweep: SweepClock::default(),
        }
    }

    /// Whether this frame draws any haze, hence needs the scene copy.
    pub fn has_work(&self) -> bool {
        !self.draws.is_empty()
    }

    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }

    /// Prepares this frame's draws. `for_each_instance` visits the resident placements. A
    /// placement whose normal maps are still being loaded is skipped until they are ready.
    pub fn prepare(
        &mut self,
        inputs: &RefractionFrameInputs,
        for_each_instance: impl FnOnce(&mut dyn FnMut(&RefractionInstance)),
    ) {
        self.collect_warmup();
        self.draws.clear();
        let planes = clip_planes(&inputs.frame.view_proj);
        let camera = inputs.frame.camera_pos.truncate();
        let forward = inputs.frame.camera_forward.truncate();
        let mut instances = std::mem::take(&mut self.instances);
        instances.clear();
        for_each_instance(&mut |instance| {
            let ready = self.material_readiness(&instance.material) == Readiness::Ready;
            if ready && !instance.bounds.is_outside(&planes) {
                instances.push(instance.clone());
            }
        });
        self.depths.clear();
        self.depths.extend(
            instances
                .iter()
                .map(|instance| view_depth(instance.bounds.center(), camera, forward)),
        );
        back_to_front(&self.depths, &mut self.order);

        let mut blocks = std::mem::take(&mut self.blocks);
        blocks.clear();
        for &index in &self.order {
            let instance = &instances[index];
            let haze = animated_haze(&instance.material, inputs.elapsed_seconds);
            let Some(block) = DrawBlock::new(
                instance.model,
                instance.material.vertex_colors,
                haze.strength,
                layer_transforms(haze.uv),
            ) else {
                continue;
            };
            self.geometry.acquire(&self.device, &instance.mesh);
            blocks.push(block);
            self.draws.push(PreparedDraw {
                double_sided: instance.material.double_sided,
                material: Arc::clone(&instance.material),
                mesh: Arc::clone(&instance.mesh),
            });
        }
        self.upload(inputs, &blocks);
        self.instances = instances;
        self.blocks = blocks;
        self.sweep_if_due();
    }

    /// Asks the worker for the bind group of `material` when it has none yet.
    fn material_readiness(&mut self, material: &Arc<RefractionMaterial>) -> Readiness {
        if self.groups.get(material).is_some() {
            Readiness::Ready
        } else if self.failed_materials.contains(material, ()) {
            Readiness::Failed
        } else {
            if self.pending_materials.insert(material, ()) {
                self.worker.submit(Arc::clone(material));
            }
            Readiness::Pending
        }
    }

    fn collect_warmup(&mut self) {
        while let Some(result) = self.worker.try_next() {
            self.accept(result);
        }
    }

    fn accept(&mut self, (material, built): MaterialWarmup) {
        self.pending_materials.remove(&material, ());
        match built {
            Ok(group) => self.groups.insert(&material, group),
            Err(error) => {
                self.failed_materials.insert(&material, ());
                warn_once(&format!("refraction-texture:{}", error.path), || {
                    format!("the heat haze is not drawn: {error}")
                });
            }
        }
    }

    #[cfg(test)]
    fn wait_for_warmup(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !self.pending_materials.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the warm-up worker did not finish"
            );
            if let Some(result) = self
                .worker
                .next_within(std::time::Duration::from_millis(50))
            {
                self.accept(result);
            }
        }
    }

    fn upload(&mut self, inputs: &RefractionFrameInputs, blocks: &[DrawBlock]) {
        self.queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&FrameBlock::new(inputs.frame, inputs.near_plane)),
        );
        self.draw_buffer
            .reserve(&self.device, &self.layouts.draw, blocks.len());
        for (index, block) in blocks.iter().enumerate() {
            self.queue.write_buffer(
                &self.draw_buffer.buffer,
                index as u64 * DRAW_STRIDE,
                bytemuck::bytes_of(block),
            );
        }
    }

    fn sweep_if_due(&mut self) {
        if self.sweep.tick() {
            self.geometry.release_unused();
            self.groups.release_unused();
            self.loader.textures.prune();
            self.pending_materials.prune();
            self.failed_materials.prune();
        }
    }

    /// Issues the prepared draws into the translucent pass. Returns whether it bound anything
    /// the caller's own bindings must be restored after.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &SceneCopy) -> bool {
        if self.draws.is_empty() {
            return false;
        }
        pass.set_bind_group(SCENE_GROUP, scene.bind_group(), &[]);
        pass.set_bind_group(FRAME_GROUP, &self.frame_group, &[]);
        for (index, draw) in self.draws.iter().enumerate() {
            let group = self
                .groups
                .get(&draw.material)
                .expect("prepare creates the bind group of every draw's material");
            let mesh = self
                .geometry
                .get(&draw.mesh)
                .expect("prepare uploads the mesh of every draw");
            pass.set_pipeline(&self.pipelines[usize::from(draw.double_sided)]);
            pass.set_bind_group(MATERIAL_GROUP, group, &[]);
            pass.set_bind_group(
                DRAW_GROUP,
                &self.draw_buffer.bind_group,
                &[(index as u64 * DRAW_STRIDE) as u32],
            );
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
        true
    }
}

fn view_depth(point: Vec3, camera: Vec3, forward: Vec3) -> f32 {
    (point - camera).dot(forward)
}

#[cfg(test)]
#[path = "gpu_tests.rs"]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn order_of(depths: &[f32]) -> Vec<usize> {
        let mut order = vec![99];
        back_to_front(depths, &mut order);
        order
    }

    #[test]
    fn refraction_draws_are_ordered_farthest_first() {
        assert_eq!(order_of(&[5.0, 20.0, 10.0]), [1, 2, 0]);
    }

    #[test]
    fn equal_depths_keep_their_order() {
        assert_eq!(order_of(&[7.0, 7.0, 7.0]), [0, 1, 2]);
    }

    #[test]
    fn no_placements_have_no_order() {
        assert_eq!(order_of(&[]), [] as [usize; 0]);
    }
}
