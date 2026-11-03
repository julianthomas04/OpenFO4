//! The loading art's GPU state that does not depend on a model: the pipeline, the per-frame
//! uniform block and the depth buffer.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use super::background_look::light_colour;
use crate::rendering::shaders;
use crate::rendering::texture::Texture;
use crate::rendering::vertex::Vertex;

/// Measured from the vanilla screen's T-51 power armour: the key comes from the viewer's upper
/// left and front, so the left silhouette is bright, the right side dark and the undersides
/// black; there is no fill or rim light.
///
/// Inferred from comparisons with a vanilla screenshot: the left and right p90 brightness differ
/// by about 1.76, the upper and lower by about 1.59, and surfaces facing the viewer's right fall
/// to black, so the key is mostly lateral and from above, barely frontal.
const LIGHT_TOWARDS: Vec3 = Vec3::new(-0.7, 0.7, 0.15);
/// Inferred: nearly none, vanilla has 6.7% pure black model pixels. The shader adds no wrap
/// light, and back faces of two-sided geometry are lit as their own side, not brightened.
const AMBIENT: f32 = 0.02;
/// Inferred: the strength of the key, aiming at a p99 luma of about 250 with 1.5 to 2% of the
/// model's pixels clipped.
const DIFFUSE: f32 = 2.5;
/// Inferred: the strength of the specular lobe of the world's surface response. The lobe is
/// white, not tinted like the diffuse light: vanilla's highlights have blue about 0.96 of green.
const SPECULAR_STRENGTH: f32 = 3.0;

/// The uniform block of `loading_art.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    light_direction: [f32; 4],
    shading: [f32; 4],
    /// The linear colour of the key, applied to its diffuse term only.
    light_colour: [f32; 4],
}

impl SceneUniform {
    pub fn new(view_proj: Mat4, model: Mat4, encode_srgb: bool) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            light_direction: LIGHT_TOWARDS.normalize().extend(0.0).to_array(),
            shading: [
                AMBIENT,
                DIFFUSE,
                if encode_srgb { 1.0 } else { 0.0 },
                SPECULAR_STRENGTH,
            ],
            light_colour: {
                let [red, green, blue] = light_colour();
                [red, green, blue, 0.0]
            },
        }
    }
}

/// The uniform block of one draw's material in `loading_art.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct MaterialUniform {
    cutoff: [f32; 4],
    surface: [f32; 4],
}

impl MaterialUniform {
    pub fn new(alpha_cutoff: f32, smoothness: f32, specular_scale: f32) -> Self {
        Self {
            cutoff: [alpha_cutoff, 0.0, 0.0, 0.0],
            surface: [smoothness, specular_scale, 0.0, 0.0],
        }
    }
}

pub(super) struct ArtPipeline {
    pub depth_prepass: wgpu::RenderPipeline,
    pub pipeline: wgpu::RenderPipeline,
    pub scene_layout: wgpu::BindGroupLayout,
    pub texture_layout: wgpu::BindGroupLayout,
}

impl ArtPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_art_scene_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_art_texture_bgl"),
            entries: &[
                filterable_texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                filterable_texture_entry(3),
                filterable_texture_entry(4),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_art_layout"),
            bind_group_layouts: &[Some(&scene_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let shader = shaders::LOADING_ART.compile(device);
        let build = |label: &str, stage: Stage| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(Vertex::desc())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(stage.entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: stage.blend,
                        write_mask: stage.write_mask,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: Texture::DEPTH_FORMAT,
                    depth_write_enabled: Some(stage.depth_write),
                    depth_compare: Some(stage.depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The nearest surface is found first, without colour, and cut-out texels write no depth;
        // the colour pass then draws only the fragments at exactly that depth, so overlapping
        // parts show their nearest surface whatever the order they are drawn in.
        let depth_prepass = build(
            "Loading Art Depth Pipeline",
            Stage {
                entry_point: "fs_depth",
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
                depth_write: true,
                depth_compare: wgpu::CompareFunction::Less,
            },
        );
        let pipeline = build(
            "Loading Art Pipeline",
            Stage {
                entry_point: "fs_main",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Equal,
            },
        );
        Self {
            depth_prepass,
            pipeline,
            scene_layout,
            texture_layout,
        }
    }
}

fn filterable_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

/// What differs between the depth prepass and the colour pass.
struct Stage {
    entry_point: &'static str,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
}

/// The depth buffer of the loading art, sized like the target it draws into.
pub(super) struct DepthTarget {
    size: (u32, u32),
    view: wgpu::TextureView,
}

impl DepthTarget {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("loading_art_depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Texture::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            size,
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// The view of a depth buffer of `size`, replacing `slot`'s when the size changed.
    pub fn view_for<'a>(
        slot: &'a mut Option<Self>,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> &'a wgpu::TextureView {
        if slot.as_ref().is_some_and(|depth| depth.size != size) {
            *slot = None;
        }
        &slot.get_or_insert_with(|| Self::new(device, size)).view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_light_is_a_unit_vector_from_the_upper_left_front() {
        let scene = SceneUniform::new(Mat4::IDENTITY, Mat4::IDENTITY, false);
        let [x, y, z, _] = scene.light_direction;

        assert!(x < 0.0 && y > 0.0 && z > 0.0, "{x} {y} {z}");
        assert!((x * x + y * y + z * z - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_shading_carries_the_ambient_the_key_the_encode_flag_and_the_specular() {
        let encoded = SceneUniform::new(Mat4::IDENTITY, Mat4::IDENTITY, true);
        let direct = SceneUniform::new(Mat4::IDENTITY, Mat4::IDENTITY, false);

        assert_eq!(encoded.shading, [AMBIENT, DIFFUSE, 1.0, SPECULAR_STRENGTH]);
        assert_eq!(direct.shading[2], 0.0);
    }

    #[test]
    fn a_material_uniform_carries_the_cutoff_the_smoothness_and_the_specular_scale() {
        let material = MaterialUniform::new(0.2, 0.7, 1.5);

        assert_eq!(material.cutoff, [0.2, 0.0, 0.0, 0.0]);
        assert_eq!(material.surface, [0.7, 1.5, 0.0, 0.0]);
    }
}
