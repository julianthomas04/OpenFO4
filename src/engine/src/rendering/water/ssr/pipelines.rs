//! Bind group layouts and pipelines of the SSR chain: three ray-setup variants,
//! the two Hi-Z compute passes, the raytrace and the two blur passes.

use std::num::NonZeroU64;

use super::targets::{ANCHOR_FORMAT, HIZ_FORMAT, REFLECTION_FORMAT, VIEW_Z_FORMAT};
use crate::rendering::{
    shaders::{self, composed},
    vertex::Vertex,
    water::pipelines::{DRAW_GROUP, FRAME_GROUP, NORMALS_GROUP, SCENE_GROUP, WaterLayouts},
};

/// Which ray-setup pixel shader draws a surface: descriptor `0x200`, `0x10200`
/// or `0x242` of the water pass selection.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RaySetupKind {
    Surface,
    Lod,
    Wading,
}

impl RaySetupKind {
    /// Every kind in draw order: surface and LOD ray setups before the wading ones.
    pub const ALL: [Self; 3] = [Self::Surface, Self::Lod, Self::Wading];

    const fn index(self) -> usize {
        self as usize
    }

    fn vertex_entry(self) -> &'static str {
        match self {
            Self::Surface => "vs_ray_setup",
            Self::Lod => "vs_ray_setup_lod",
            Self::Wading => "vs_ray_setup_wading",
        }
    }

    fn fragment_entry(self) -> &'static str {
        match self {
            Self::Surface => "fs_ray_setup",
            Self::Lod => "fs_ray_setup_lod",
            Self::Wading => "fs_ray_setup_wading",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Surface => "water_ssr_ray_setup_surface",
            Self::Lod => "water_ssr_ray_setup_lod",
            Self::Wading => "water_ssr_ray_setup_wading",
        }
    }
}

const VEC4_BYTES: u64 = std::mem::size_of::<[f32; 4]>() as u64;

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn uniform_vec4(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        visibility,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(VEC4_BYTES),
        },
    )
}

fn float_texture(
    binding: u32,
    visibility: wgpu::ShaderStages,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        visibility,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::ShaderStages::COMPUTE,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: HIZ_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}

/// Binding numbers of the SSR shaders, shared with the bind groups.
pub mod binding {
    pub const HIZ_DEPTH: u32 = 0;
    pub const HIZ_LINEAR_Z: u32 = 1;
    pub const HIZ_MIP0: u32 = 2;
    pub const HIZ_MIP1: u32 = 3;
    pub const HIZ_MIP2: u32 = 4;
    pub const HIZ_MIP2_IN: u32 = 5;
    pub const HIZ_MIP3: u32 = 6;
    pub const HIZ_MIP4: u32 = 7;

    pub const RAYTRACE_CB0: u32 = 0;
    pub const RAYTRACE_HIZ: u32 = 1;
    pub const RAYTRACE_ANCHOR: u32 = 2;
    pub const RAYTRACE_VIEW_Z: u32 = 3;
    pub const RAYTRACE_POINT_SAMPLER: u32 = 4;

    pub const BLUR_CB0: u32 = 0;
    pub const BLUR_SOURCE: u32 = 1;
    pub const BLUR_SAMPLER: u32 = 2;
}

/// The bind group layouts the SSR passes own (the ray setup shares the water
/// layouts, the raytrace shares the scene copy's group 0).
pub struct SsrLayouts {
    pub hiz_camera: wgpu::BindGroupLayout,
    pub hiz_mips: wgpu::BindGroupLayout,
    pub raytrace: wgpu::BindGroupLayout,
    pub blur: wgpu::BindGroupLayout,
}

impl SsrLayouts {
    fn new(device: &wgpu::Device) -> Self {
        use wgpu::ShaderStages as Stage;
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        Self {
            hiz_camera: layout(
                "water_ssr_hiz_camera_layout",
                &[
                    entry(
                        binding::HIZ_DEPTH,
                        Stage::COMPUTE,
                        wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                    ),
                    uniform_vec4(binding::HIZ_LINEAR_Z, Stage::COMPUTE),
                    storage_texture(binding::HIZ_MIP0),
                    storage_texture(binding::HIZ_MIP1),
                    storage_texture(binding::HIZ_MIP2),
                ],
            ),
            hiz_mips: layout(
                "water_ssr_hiz_mips_layout",
                &[
                    float_texture(binding::HIZ_MIP2_IN, Stage::COMPUTE, false),
                    storage_texture(binding::HIZ_MIP3),
                    storage_texture(binding::HIZ_MIP4),
                ],
            ),
            raytrace: layout(
                "water_ssr_raytrace_layout",
                &[
                    uniform_vec4(binding::RAYTRACE_CB0, Stage::FRAGMENT),
                    float_texture(binding::RAYTRACE_HIZ, Stage::FRAGMENT, false),
                    float_texture(binding::RAYTRACE_ANCHOR, Stage::FRAGMENT, true),
                    float_texture(binding::RAYTRACE_VIEW_Z, Stage::FRAGMENT, false),
                    entry(
                        binding::RAYTRACE_POINT_SAMPLER,
                        Stage::FRAGMENT,
                        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    ),
                ],
            ),
            blur: layout(
                "water_ssr_blur_layout",
                &[
                    uniform_vec4(binding::BLUR_CB0, Stage::VERTEX),
                    float_texture(binding::BLUR_SOURCE, Stage::FRAGMENT, true),
                    entry(
                        binding::BLUR_SAMPLER,
                        Stage::FRAGMENT,
                        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    ),
                ],
            ),
        }
    }
}

pub struct SsrPipelines {
    pub layouts: SsrLayouts,
    ray_setup: [wgpu::RenderPipeline; 3],
    hiz_camera: wgpu::ComputePipeline,
    hiz_mips: wgpu::ComputePipeline,
    raytrace: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
}

fn color_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        strip_index_format: None,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: None,
        polygon_mode: wgpu::PolygonMode::Fill,
        unclipped_depth: false,
        conservative: false,
    }
}

struct RenderSpec<'a> {
    label: &'static str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vertex_entry: &'static str,
    vertex_buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    fragment_entry: &'static str,
    targets: &'a [Option<wgpu::ColorTargetState>],
}

fn render_pipeline(device: &wgpu::Device, spec: &RenderSpec) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.module,
            entry_point: Some(spec.vertex_entry),
            buffers: spec.vertex_buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: spec.module,
            entry_point: Some(spec.fragment_entry),
            targets: spec.targets,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: primitive(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn compute_pipeline(
    device: &wgpu::Device,
    label: &'static str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry_point: &'static str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        module,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

impl SsrPipelines {
    pub fn new(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        water_layouts: &WaterLayouts,
    ) -> Self {
        let layouts = SsrLayouts::new(device);
        let pipeline_layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };

        let ray_setup_module =
            shaders::composed::water::WATER_SSR_RAY_SETUP.compile_runtime(device);
        let ray_setup_layout = {
            let mut groups = [None; 4];
            groups[SCENE_GROUP as usize] = Some(scene_layout);
            groups[FRAME_GROUP as usize] = Some(&water_layouts.frame);
            groups[NORMALS_GROUP as usize] = Some(&water_layouts.normals);
            groups[DRAW_GROUP as usize] = Some(&water_layouts.draw);
            pipeline_layout("water_ssr_ray_setup_layout", &groups)
        };
        let ray_setup_targets = [
            Some(color_target(ANCHOR_FORMAT)),
            Some(color_target(VIEW_Z_FORMAT)),
        ];
        let ray_setup = RaySetupKind::ALL.map(|kind| {
            render_pipeline(
                device,
                &RenderSpec {
                    label: kind.label(),
                    layout: &ray_setup_layout,
                    module: &ray_setup_module,
                    vertex_entry: kind.vertex_entry(),
                    vertex_buffers: &[Some(Vertex::desc())],
                    fragment_entry: kind.fragment_entry(),
                    targets: &ray_setup_targets,
                },
            )
        });

        let hiz_module = composed::ssr::WATER_SSR_HIZ.compile_runtime(device);
        let hiz_camera = compute_pipeline(
            device,
            "water_ssr_hiz_camera",
            &pipeline_layout(
                "water_ssr_hiz_camera_pipeline_layout",
                &[Some(&layouts.hiz_camera)],
            ),
            &hiz_module,
            "hiz_camera_and_mips",
        );
        let hiz_mips = compute_pipeline(
            device,
            "water_ssr_hiz_mips",
            &pipeline_layout(
                "water_ssr_hiz_mips_pipeline_layout",
                &[Some(&layouts.hiz_mips)],
            ),
            &hiz_module,
            "hiz_mips",
        );

        let raytrace_module = composed::ssr::WATER_SSR_RAYTRACE.compile_runtime(device);
        let raytrace = render_pipeline(
            device,
            &RenderSpec {
                label: "water_ssr_raytrace",
                layout: &pipeline_layout(
                    "water_ssr_raytrace_pipeline_layout",
                    &[Some(scene_layout), Some(&layouts.raytrace)],
                ),
                module: &raytrace_module,
                vertex_entry: "vs_fullscreen",
                vertex_buffers: &[],
                fragment_entry: "fs_raytrace",
                targets: &[Some(color_target(REFLECTION_FORMAT))],
            },
        );

        let blur_module = composed::ssr::WATER_SSR_BLUR.compile_runtime(device);
        let blur_layout = pipeline_layout("water_ssr_blur_pipeline_layout", &[Some(&layouts.blur)]);
        let blur = |label, vertex_entry| {
            render_pipeline(
                device,
                &RenderSpec {
                    label,
                    layout: &blur_layout,
                    module: &blur_module,
                    vertex_entry,
                    vertex_buffers: &[],
                    fragment_entry: "fs_blur",
                    targets: &[Some(color_target(REFLECTION_FORMAT))],
                },
            )
        };
        Self {
            ray_setup,
            hiz_camera,
            hiz_mips,
            raytrace,
            blur_h: blur("water_ssr_blur_h", "vs_blur_h"),
            blur_v: blur("water_ssr_blur_v", "vs_blur_v"),
            layouts,
        }
    }

    pub fn ray_setup(&self, kind: RaySetupKind) -> &wgpu::RenderPipeline {
        &self.ray_setup[kind.index()]
    }

    pub fn hiz_camera(&self) -> &wgpu::ComputePipeline {
        &self.hiz_camera
    }

    pub fn hiz_mips(&self) -> &wgpu::ComputePipeline {
        &self.hiz_mips
    }

    pub fn raytrace(&self) -> &wgpu::RenderPipeline {
        &self.raytrace
    }

    pub fn blur_h(&self) -> &wgpu::RenderPipeline {
        &self.blur_h
    }

    pub fn blur_v(&self) -> &wgpu::RenderPipeline {
        &self.blur_v
    }
}
