//! The stage on the GPU: each width's planes with their textures, drawn by
//! `loading_stage.wgsl` into the loading screen's linear picture with the blend their materials
//! name, from where the stage's planes put the camera.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::camera::perspective;
use super::grade::INFERRED_LIFT;
use super::stage::{CpuStage, StageError, StageLayer, keep_usable, nearest_aspect};
use super::stage_framing::StageFraming;
use crate::assets::layout::named_texture_key;
use crate::assets::types::effect_material::{ClampMode, EffectMaterial, EffectTextures};
use crate::rendering::effects::pipeline::{BlendFactors, blend_factors, blend_state};
use crate::rendering::shaders;
use crate::rendering::texture::load_texture_asset_srgb;
use crate::rendering::vertex::Vertex;

/// The uniform block of one plane's material in `loading_stage.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct StageMaterialUniform {
    colour: [f32; 4],
    /// `x`: the lift added to the plane.
    lift: [f32; 4],
}

impl StageMaterialUniform {
    /// An effect shape's colour: its base colour times its scale, and the base colour's alpha
    /// times the material's. A plane that blends by its source alpha, as the additive overlay
    /// does, takes the base colour's alpha twice: an interpretation confirmed against a vanilla
    /// capture within about 8%, not documented data.
    fn new(material: &EffectMaterial, lift: f32) -> Self {
        let [red, green, blue, alpha] = material.base_color;
        let scale = material.base_color_scale;
        let blends_by_source_alpha = blend_factors(&material.blend)
            .is_some_and(|(source, _)| source == wgpu::BlendFactor::SrcAlpha);
        let base_alpha = if blends_by_source_alpha {
            alpha * alpha
        } else {
            alpha
        };
        Self {
            colour: [
                red * scale,
                green * scale,
                blue * scale,
                base_alpha * material.alpha,
            ],
            lift: [lift, 0.0, 0.0, 0.0],
        }
    }
}

struct StageDraw {
    indices: Range<u32>,
    base_vertex: i32,
    pipeline: usize,
    group: wgpu::BindGroup,
}

/// One width of the stage on the GPU.
struct GpuStage {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<StageDraw>,
    framing: StageFraming,
    aspect: f32,
}

/// Every width of the stage the install has.
pub struct LoadedStage {
    stages: Vec<GpuStage>,
}

impl LoadedStage {
    /// The framing of the width drawn on a target of `aspect`.
    pub fn framing_for(&self, aspect: f32) -> Option<StageFraming> {
        self.for_aspect(aspect).map(|stage| stage.framing)
    }

    fn for_aspect(&self, aspect: f32) -> Option<&GpuStage> {
        let aspects: Vec<f32> = self.stages.iter().map(|stage| stage.aspect).collect();
        nearest_aspect(&aspects, aspect).map(|index| &self.stages[index])
    }
}

pub struct StageRenderer {
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    material_layout: wgpu::BindGroupLayout,
    view_buffer: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    target_format: wgpu::TextureFormat,
    /// One pipeline per blend the stages' materials name; `None` draws opaque.
    pipelines: Vec<(Option<BlendFactors>, wgpu::RenderPipeline)>,
}

impl StageRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_stage_view_bgl"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_stage_material_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_stage_layout"),
            bind_group_layouts: &[Some(&view_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading_stage_view"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading_stage_view_group"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });
        Self {
            shader: shaders::LOADING_STAGE.compile(device),
            layout,
            material_layout,
            view_buffer,
            view_group,
            target_format,
            pipelines: Vec::new(),
        }
    }

    /// Puts `stages` on the GPU with the textures their materials name below `asset_root`. A
    /// width with a plane that cannot be drawn is left out, logged; an error when none is left.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stages: &[CpuStage],
        asset_root: &Path,
    ) -> Result<LoadedStage, StageError> {
        let mut textures = HashMap::new();
        let stages = keep_usable(
            stages
                .iter()
                .map(|stage| self.upload_stage(device, queue, stage, asset_root, &mut textures)),
        )?;
        Ok(LoadedStage { stages })
    }

    fn upload_stage(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stage: &CpuStage,
        asset_root: &Path,
        textures: &mut TextureCache,
    ) -> Result<GpuStage, StageError> {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        for layer in &stage.layers {
            let base_vertex = i32::try_from(vertices.len()).unwrap_or(i32::MAX);
            let first = u32::try_from(indices.len()).unwrap_or(u32::MAX);
            vertices.extend_from_slice(&layer.vertices);
            indices.extend_from_slice(&layer.indices);
            let last = u32::try_from(indices.len()).unwrap_or(u32::MAX);
            let texture = layer_texture(device, queue, stage, layer, asset_root, textures)?;
            draws.push(StageDraw {
                indices: first..last,
                base_vertex,
                pipeline: self.pipeline_for(device, blend_factors(&layer.material.blend)),
                group: self.material_group(device, layer, &texture),
            });
        }
        Ok(GpuStage {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("loading_stage_vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("loading_stage_indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws,
            framing: stage.framing,
            aspect: stage.aspect,
        })
    }

    fn material_group(
        &self,
        device: &wgpu::Device,
        layer: &StageLayer,
        texture: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let material = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("loading_stage_material"),
            contents: bytemuck::bytes_of(&StageMaterialUniform::new(
                &layer.material,
                if layer.is_opaque() {
                    INFERRED_LIFT
                } else {
                    0.0
                },
            )),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (address_mode_u, address_mode_v) = address_modes(layer.material.clamp);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("loading_stage_sampler"),
            address_mode_u,
            address_mode_v,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading_stage_material_group"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: material.as_entire_binding(),
                },
            ],
        })
    }

    fn pipeline_for(&mut self, device: &wgpu::Device, blend: Option<BlendFactors>) -> usize {
        if let Some(index) = self.pipelines.iter().position(|(known, _)| *known == blend) {
            return index;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loading Stage Pipeline"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(Vertex::desc())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: blend.map(blend_state),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.push((blend, pipeline));
        self.pipelines.len() - 1
    }

    /// Draws the width of `stage` nearest the target's `aspect` into `pass`.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        stage: &LoadedStage,
        aspect: f32,
    ) {
        let Some(stage) = stage.for_aspect(aspect) else {
            return;
        };
        let view_proj =
            perspective(stage.framing.vertical_fov_radians, aspect) * stage.framing.stage_view();
        queue.write_buffer(
            &self.view_buffer,
            0,
            bytemuck::bytes_of(&view_proj.to_cols_array_2d()),
        );
        pass.set_bind_group(0, &self.view_group, &[]);
        pass.set_vertex_buffer(0, stage.vertices.slice(..));
        pass.set_index_buffer(stage.indices.slice(..), wgpu::IndexFormat::Uint32);
        for draw in &stage.draws {
            pass.set_pipeline(&self.pipelines[draw.pipeline].1);
            pass.set_bind_group(1, &draw.group, &[]);
            pass.draw_indexed(draw.indices.clone(), draw.base_vertex, 0..1);
        }
    }
}

/// The views of the textures loaded so far by key, or why one did not load.
type TextureCache = HashMap<String, Result<wgpu::TextureView, String>>;

/// The view of the plane's source texture, each texture loaded once. An error when the material
/// names none or it does not load: the stage cannot be drawn without it.
fn layer_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    stage: &CpuStage,
    layer: &StageLayer,
    asset_root: &Path,
    textures: &mut TextureCache,
) -> Result<wgpu::TextureView, StageError> {
    let Some(source) = EffectTextures::used(&layer.material.textures.source) else {
        return Err(StageError::PlaneWithoutTexture {
            path: stage.path.clone(),
        });
    };
    let key = named_texture_key(source);
    textures
        .entry(key.clone())
        .or_insert_with(|| {
            load_texture_asset_srgb(device, queue, &asset_root.to_string_lossy(), &key)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(|reason| StageError::PlaneTextureUnavailable {
            path: stage.path.clone(),
            reason,
        })
}

fn address_modes(clamp: ClampMode) -> (wgpu::AddressMode, wgpu::AddressMode) {
    use wgpu::AddressMode::{ClampToEdge, Repeat};
    match clamp {
        ClampMode::ClampSClampT => (ClampToEdge, ClampToEdge),
        ClampMode::ClampSWrapT => (ClampToEdge, Repeat),
        ClampMode::WrapSClampT => (Repeat, ClampToEdge),
        ClampMode::WrapSWrapT => (Repeat, Repeat),
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the products below are of exactly representable values"
)]
mod tests {
    use super::*;
    use crate::assets::types::effect_material::{Blend, BlendMode};

    #[test]
    fn a_material_colour_is_its_scaled_base_colour_and_its_combined_alpha() {
        let mut material: EffectMaterial = serde_json::from_value(
            crate::rendering::loading_screen::test_support::TestStageLayer::vanilla()[0]
                .effect_json(),
        )
        .unwrap();
        material.base_color = [0.5, 0.25, 1.0, 0.6];
        material.base_color_scale = 2.0;
        material.alpha = 0.5;
        material.blend = Blend {
            mode: BlendMode::Mult,
            src: None,
            dst: None,
            ..material.blend
        };

        let uniform = StageMaterialUniform::new(&material, 0.0);

        assert_eq!(uniform.colour[..3], [1.0, 0.5, 2.0]);
        assert!((uniform.colour[3] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_material_blending_by_source_alpha_takes_its_base_colour_alpha_twice() {
        let mut material: EffectMaterial = serde_json::from_value(
            crate::rendering::loading_screen::test_support::TestStageLayer::vanilla()[0]
                .effect_json(),
        )
        .unwrap();
        material.base_color = [1.0, 1.0, 1.0, 0.6];
        material.alpha = 1.0;
        material.blend.mode = BlendMode::Add;

        let uniform = StageMaterialUniform::new(&material, 0.0);

        assert!((uniform.colour[3] - 0.36).abs() < 1e-6);
    }

    #[test]
    fn the_clamp_mode_picks_the_address_mode_per_axis() {
        use wgpu::AddressMode::{ClampToEdge, Repeat};
        assert_eq!(address_modes(ClampMode::WrapSWrapT), (Repeat, Repeat));
        assert_eq!(address_modes(ClampMode::ClampSWrapT), (ClampToEdge, Repeat));
        assert_eq!(address_modes(ClampMode::WrapSClampT), (Repeat, ClampToEdge));
        assert_eq!(
            address_modes(ClampMode::ClampSClampT),
            (ClampToEdge, ClampToEdge)
        );
    }
}
