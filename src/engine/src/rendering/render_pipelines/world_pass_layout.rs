use std::num::NonZeroU32;

use crate::rendering::{
    cube_array::CUBE_SLOTS, device::STORAGE_BUFFERS_PER_STAGE, gpu_driven::MAX_TEXTURES,
    lighting::frame_binding::FRAME_STORAGE_BUFFERS,
};

/// Fragment-stage storage buffers of the world pass: materials, texture first
/// mips, texture feedback, object and terrain instances, plus the frame
/// group's local-light buffers (counted for the main-view object and terrain
/// pipelines, the only ones that bind the feedback group).
const FRAGMENT_STORAGE_BUFFERS: u32 = 5 + FRAME_STORAGE_BUFFERS;
const _: () = assert!(FRAGMENT_STORAGE_BUFFERS <= STORAGE_BUFFERS_PER_STAGE);

/// The single pipeline layout of every pipeline drawn in the world pass. The
/// pass binds all groups once up front; a shared layout keeps those bindings
/// valid across every pipeline switch, whichever groups a shader reads.
/// `feedback_pipeline_layout` adds the writable texture feedback group for the
/// object and terrain pipelines; everything else (shadows included) keeps the
/// read-only layout.
pub struct WorldPassLayout {
    pub material_bind_group_layout: wgpu::BindGroupLayout,
    pub instance_bind_group_layout: wgpu::BindGroupLayout,
    pub feedback_bind_group_layout: wgpu::BindGroupLayout,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub feedback_pipeline_layout: wgpu::PipelineLayout,
}

pub const FEEDBACK_GROUP: u32 = 3;

impl WorldPassLayout {
    pub fn new(device: &wgpu::Device, frame_bind_group_layout: &wgpu::BindGroupLayout) -> Self {
        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("material_bgl"),
                entries: &[
                    // 0: texture binding array
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: NonZeroU32::new(MAX_TEXTURES),
                    },
                    // 1: material sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // 2: material storage buffer
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 3: terrain sampler (needed for terrain pass sharing the same group 0)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // 4: splatmap array (terrain pass; one layer per landscape cell)
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    // 5: shadow alpha-test sampler (trilinear, no anisotropy)
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // 6: resident first mip per texture slot (mip debug views)
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 7: environment cube binding array (slot 0 is black)
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: NonZeroU32::new(CUBE_SLOTS),
                    },
                ],
            });

        let instance_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instance_bgl"),
                entries: &[
                    // 0: object instances
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 1: terrain instances
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World Pass Pipeline Layout"),
            bind_group_layouts: &[
                Some(&material_bind_group_layout),
                Some(frame_bind_group_layout),
                Some(&instance_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let feedback_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("texture_feedback_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let feedback_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("World Pass Feedback Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&material_bind_group_layout),
                    Some(frame_bind_group_layout),
                    Some(&instance_bind_group_layout),
                    Some(&feedback_bind_group_layout),
                ],
                immediate_size: 0,
            });

        Self {
            material_bind_group_layout,
            instance_bind_group_layout,
            feedback_bind_group_layout,
            pipeline_layout,
            feedback_pipeline_layout,
        }
    }
}
