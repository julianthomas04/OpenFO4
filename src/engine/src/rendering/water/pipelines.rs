//! Bind group layouts and pipelines of the water shader. The group numbers
//! are those of `shaders/water_wrapper.wgsl`: 0 scene copy, 1 frame constants, stand-in
//! textures and the water mask, 2 the WATR's normal layers, 3 the per-draw
//! constants.

use std::collections::HashMap;
use std::num::NonZeroU64;

use super::{
    constants::{DRAW_CONSTANTS_STRIDE, WaterFrameConstants},
    debug_output::{DrawPipeline, WaterDebugOutput},
    permutation::{MaskTag, Variant},
    textures::{MASK_FORMAT, ReflectionViews, SURFACE_DEPTH_FORMAT, StandInTextures, WaterMask},
};
use crate::rendering::{post::hdr_target::HDR_FORMAT, shaders, texture::Texture, vertex::Vertex};

pub const SCENE_GROUP: u32 = 0;
pub const FRAME_GROUP: u32 = 1;
pub const NORMALS_GROUP: u32 = 2;
pub const DRAW_GROUP: u32 = 3;

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32, dynamic: bool, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

const FRAME_UNIFORM_BINDING: u32 = 0;
const RIPPLE_BINDING: u32 = 1;
const SSR_SHARP_BINDING: u32 = 2;
const SSR_BLURRED_BINDING: u32 = 3;
const CLAMP_SAMPLER_BINDING: u32 = 4;
const MASK_BINDING: u32 = 5;

pub struct WaterLayouts {
    pub frame: wgpu::BindGroupLayout,
    pub normals: wgpu::BindGroupLayout,
    pub draw: wgpu::BindGroupLayout,
}

impl WaterLayouts {
    pub fn new(device: &wgpu::Device) -> Self {
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_frame_layout"),
            entries: &[
                uniform_entry(
                    FRAME_UNIFORM_BINDING,
                    false,
                    std::mem::size_of::<WaterFrameConstants>() as u64,
                ),
                texture_entry(RIPPLE_BINDING),
                texture_entry(SSR_SHARP_BINDING),
                texture_entry(SSR_BLURRED_BINDING),
                sampler_entry(CLAMP_SAMPLER_BINDING),
                texture_entry(MASK_BINDING),
            ],
        });
        let normals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_normals_layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let draw = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_draw_layout"),
            entries: &[uniform_entry(0, true, DRAW_CONSTANTS_STRIDE)],
        });
        Self {
            frame,
            normals,
            draw,
        }
    }
}

/// The frame group: per-frame constants, the stand-in textures, the SSR
/// textures and the water mask. The mask follows the render target size, so the
/// bind groups are rebuilt when [`Self::ensure_target_size`] recreates it or
/// when [`Self::bind_reflection`] swaps the SSR textures.
///
/// A second group over the same textures holds the SSR ray setup's own copy of
/// the constants (half-resolution texel size, view rotation, projection); it
/// binds the empty reflection stand-ins because the ray setup runs while the SSR
/// textures are render targets.
pub struct FrameBindings {
    buffer: wgpu::Buffer,
    ray_setup_buffer: wgpu::Buffer,
    stand_ins: StandInTextures,
    reflection: ReflectionViews,
    clamp_sampler: wgpu::Sampler,
    mask: WaterMask,
    /// The displacement targets, bound as `t2` by [`Self::wading_group`].
    displacement: [wgpu::TextureView; 2],
    group: wgpu::BindGroup,
    /// The same group with `t2` reading displacement target `i`.
    wading_groups: [wgpu::BindGroup; 2],
    ray_setup_group: wgpu::BindGroup,
    /// The ray setup group with `t2` reading displacement target `i`, for the
    /// wading ray setup (descriptor `0x242`) that samples the ripple.
    ray_setup_wading_groups: [wgpu::BindGroup; 2],
}

fn constants_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: std::mem::size_of::<WaterFrameConstants>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl FrameBindings {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layouts: &WaterLayouts,
        width: u32,
        height: u32,
        displacement: &[wgpu::TextureView; 2],
    ) -> Self {
        let buffer = constants_buffer(device, "water_frame_constants");
        let ray_setup_buffer = constants_buffer(device, "water_ssr_ray_setup_frame_constants");
        let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water_clamp_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let stand_ins = StandInTextures::new(device, queue);
        let reflection = stand_ins.reflection();
        let mask = WaterMask::new(device, width, height);
        Self {
            group: Self::build_group(
                device,
                layouts,
                &buffer,
                &stand_ins.ripple,
                &reflection,
                &clamp_sampler,
                &mask,
            ),
            wading_groups: [0, 1].map(|index| {
                Self::build_group(
                    device,
                    layouts,
                    &buffer,
                    &displacement[index],
                    &reflection,
                    &clamp_sampler,
                    &mask,
                )
            }),
            ray_setup_group: Self::build_group(
                device,
                layouts,
                &ray_setup_buffer,
                &stand_ins.ripple,
                &stand_ins.reflection(),
                &clamp_sampler,
                &mask,
            ),
            ray_setup_wading_groups: [0, 1].map(|index| {
                Self::build_group(
                    device,
                    layouts,
                    &ray_setup_buffer,
                    &displacement[index],
                    &stand_ins.reflection(),
                    &clamp_sampler,
                    &mask,
                )
            }),
            displacement: displacement.clone(),
            buffer,
            ray_setup_buffer,
            stand_ins,
            reflection,
            clamp_sampler,
            mask,
        }
    }

    /// The frame group of a surface that reads displacement target `target`.
    pub fn wading_group(&self, target: usize) -> &wgpu::BindGroup {
        &self.wading_groups[target]
    }

    fn rebuild_groups(&mut self, device: &wgpu::Device, layouts: &WaterLayouts) {
        let no_reflection = self.stand_ins.reflection();
        let build =
            |buffer: &wgpu::Buffer, ripple: &wgpu::TextureView, reflection: &ReflectionViews| {
                Self::build_group(
                    device,
                    layouts,
                    buffer,
                    ripple,
                    reflection,
                    &self.clamp_sampler,
                    &self.mask,
                )
            };
        self.group = build(&self.buffer, &self.stand_ins.ripple, &self.reflection);
        self.wading_groups =
            [0, 1].map(|index| build(&self.buffer, &self.displacement[index], &self.reflection));
        self.ray_setup_group = build(
            &self.ray_setup_buffer,
            &self.stand_ins.ripple,
            &no_reflection,
        );
        self.ray_setup_wading_groups = [0, 1].map(|index| {
            build(
                &self.ray_setup_buffer,
                &self.displacement[index],
                &no_reflection,
            )
        });
    }

    fn build_group(
        device: &wgpu::Device,
        layouts: &WaterLayouts,
        buffer: &wgpu::Buffer,
        ripple: &wgpu::TextureView,
        reflection: &ReflectionViews,
        clamp_sampler: &wgpu::Sampler,
        mask: &WaterMask,
    ) -> wgpu::BindGroup {
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_frame_group"),
            layout: &layouts.frame,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: FRAME_UNIFORM_BINDING,
                    resource: buffer.as_entire_binding(),
                },
                view(RIPPLE_BINDING, ripple),
                view(SSR_SHARP_BINDING, &reflection.sharp),
                view(SSR_BLURRED_BINDING, &reflection.blurred),
                wgpu::BindGroupEntry {
                    binding: CLAMP_SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(clamp_sampler),
                },
                view(MASK_BINDING, mask.view()),
            ],
        })
    }

    /// Resizes the mask to the render target; the bind groups are rebuilt only
    /// when the size changed.
    pub fn ensure_target_size(
        &mut self,
        device: &wgpu::Device,
        layouts: &WaterLayouts,
        width: u32,
        height: u32,
    ) {
        if self.mask.ensure_size(device, width, height) {
            self.rebuild_groups(device, layouts);
        }
    }

    /// Binds the SSR chain's sharp and blurred reflections as `t9` / `t10`, or the empty stand-ins
    /// for `None`. Rebuilds the groups only when the views differ.
    pub fn bind_reflection(
        &mut self,
        device: &wgpu::Device,
        layouts: &WaterLayouts,
        views: Option<ReflectionViews>,
    ) {
        let wanted = views.unwrap_or_else(|| self.stand_ins.reflection());
        if wanted.sharp == self.reflection.sharp && wanted.blurred == self.reflection.blurred {
            return;
        }
        self.reflection = wanted;
        self.rebuild_groups(device, layouts);
    }

    pub fn write(&self, queue: &wgpu::Queue, constants: &WaterFrameConstants) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(constants));
    }

    pub fn write_ray_setup(&self, queue: &wgpu::Queue, constants: &WaterFrameConstants) {
        queue.write_buffer(&self.ray_setup_buffer, 0, bytemuck::bytes_of(constants));
    }

    pub fn group(&self) -> &wgpu::BindGroup {
        &self.group
    }

    pub fn ray_setup_group(&self) -> &wgpu::BindGroup {
        &self.ray_setup_group
    }

    /// The ray setup group of a surface that reads displacement target `target`.
    pub fn ray_setup_wading_group(&self, target: usize) -> &wgpu::BindGroup {
        &self.ray_setup_wading_groups[target]
    }

    pub fn mask_view(&self) -> &wgpu::TextureView {
        self.mask.view()
    }
}

/// A debug output draws through the vertex entry of the shading draw it replaces.
type DebugKey = (WaterDebugOutput, &'static str);

/// The depth-fog pass shares its vertex entry with the mask and surface depth passes.
const POSITION_VERTEX_ENTRY: &str = "vs_position";

/// The water pipelines: the shading permutations, the depth-fog colour pass,
/// the two mask-tag passes, the underwater surface depth pass and, once a water
/// debug view has been selected, the pipelines of its output.
pub struct WaterPipelines {
    shading: [wgpu::RenderPipeline; Variant::COUNT],
    depth_fog: wgpu::RenderPipeline,
    mask_surface: wgpu::RenderPipeline,
    mask_wading: wgpu::RenderPipeline,
    surface_depth: wgpu::RenderPipeline,
    module: wgpu::ShaderModule,
    no_normals_layout: wgpu::PipelineLayout,
    /// Created the first time an output is selected by [`Self::ensure_debug`]: a debug view is
    /// a diagnostic, and a pipeline per vertex entry and output would lengthen every start.
    debug: HashMap<DebugKey, wgpu::RenderPipeline>,
}

impl WaterPipelines {
    pub fn new(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        layouts: &WaterLayouts,
    ) -> Self {
        let module = shaders::composed::water::WATER.compile_runtime(device);
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water_pipeline_layout"),
            bind_group_layouts: &[
                Some(scene_layout),
                Some(&layouts.frame),
                Some(&layouts.normals),
                Some(&layouts.draw),
            ],
            immediate_size: 0,
        });
        // The depth-fog pass and the debug views read no normal layers.
        let no_normals_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water_no_normals_pipeline_layout"),
            bind_group_layouts: &[
                Some(scene_layout),
                Some(&layouts.frame),
                None,
                Some(&layouts.draw),
            ],
            immediate_size: 0,
        });
        // The mask passes only transform positions: no scene, no frame group
        // (the mask itself is their render target).
        let mask_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water_mask_pipeline_layout"),
            bind_group_layouts: &[None, None, None, Some(&layouts.draw)],
            immediate_size: 0,
        });

        let pipeline = |spec: PipelineSpec| create_pipeline(device, &module, &spec);
        // Depth fog tints the opaque scene by the water body's colour and alpha.
        let tint_scene = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: KEEP_DESTINATION,
        };
        let shading = Variant::ALL.map(|variant| {
            pipeline(PipelineSpec {
                label: "water_shading",
                layout: &main_layout,
                vertex_entry: vertex_entry_of(variant),
                fragment_entry: fragment_entry_of(variant),
                format: HDR_FORMAT,
                blend: Some(REPLACE_COLOUR),
            })
        });
        let mask_pass = |fragment_entry| {
            pipeline(PipelineSpec {
                label: "water_mask",
                layout: &mask_layout,
                vertex_entry: POSITION_VERTEX_ENTRY,
                fragment_entry,
                format: MASK_FORMAT,
                blend: Some(wgpu::BlendState::REPLACE),
            })
        };
        Self {
            shading,
            depth_fog: pipeline(PipelineSpec {
                label: "water_depth_fog",
                layout: &no_normals_layout,
                vertex_entry: POSITION_VERTEX_ENTRY,
                fragment_entry: "fs_depth_fog",
                format: HDR_FORMAT,
                blend: Some(tint_scene),
            }),
            mask_surface: mask_pass("fs_mask_surface"),
            mask_wading: mask_pass("fs_mask_wading"),
            // A 32-bit float target is not blendable: the depth is written plain.
            surface_depth: pipeline(PipelineSpec {
                label: "water_surface_depth",
                layout: &mask_layout,
                vertex_entry: POSITION_VERTEX_ENTRY,
                fragment_entry: "fs_surface_depth",
                format: SURFACE_DEPTH_FORMAT,
                blend: None,
            }),
            module,
            no_normals_layout,
            debug: HashMap::new(),
        }
    }

    /// Creates the pipelines `output` draws through, unless they exist. The debug pipelines
    /// write the colour like the shading ones (the depth-fog output over black, see
    /// [`OVER_BLACK`]); only the depth-fog output has a vertex entry of its own, the others
    /// follow the shading draw they replace.
    pub(super) fn ensure_debug(&mut self, device: &wgpu::Device, output: WaterDebugOutput) {
        for variant in Variant::ALL {
            let vertex_entry = debug_vertex_entry(output, variant);
            self.debug.entry((output, vertex_entry)).or_insert_with(|| {
                create_pipeline(
                    device,
                    &self.module,
                    &PipelineSpec {
                        label: "water_debug",
                        layout: &self.no_normals_layout,
                        vertex_entry,
                        fragment_entry: output.fragment_entry(),
                        format: HDR_FORMAT,
                        blend: Some(debug_blend(output)),
                    },
                )
            });
        }
    }

    /// The pipeline of a draw. A debug pipeline exists once [`Self::ensure_debug`] has run for
    /// its output, which the stage does when the output is selected.
    pub(super) fn pipeline(&self, draw: DrawPipeline) -> &wgpu::RenderPipeline {
        match draw {
            DrawPipeline::Shading(variant) => self.shading(variant),
            DrawPipeline::Debug(output, variant) => self.debug(output, variant),
        }
    }

    fn debug(&self, output: WaterDebugOutput, variant: Variant) -> &wgpu::RenderPipeline {
        self.debug
            .get(&(output, debug_vertex_entry(output, variant)))
            .expect("the stage ensures the pipelines of a selected debug output")
    }

    pub fn shading(&self, variant: Variant) -> &wgpu::RenderPipeline {
        &self.shading[variant.index()]
    }

    pub fn depth_fog(&self) -> &wgpu::RenderPipeline {
        &self.depth_fog
    }

    pub fn surface_depth(&self) -> &wgpu::RenderPipeline {
        &self.surface_depth
    }

    pub fn mask(&self, tag: MaskTag) -> &wgpu::RenderPipeline {
        match tag {
            MaskTag::Surface => &self.mask_surface,
            MaskTag::Wading => &self.mask_wading,
        }
    }
}

fn debug_vertex_entry(output: WaterDebugOutput, variant: Variant) -> &'static str {
    match output {
        WaterDebugOutput::DepthFog => POSITION_VERTEX_ENTRY,
        _ => vertex_entry_of(variant),
    }
}

fn vertex_entry_of(variant: Variant) -> &'static str {
    match (variant.underwater, variant.lod, variant.ssr, variant.depth) {
        (true, _, _, _) => "vs_underwater",
        (false, true, true, _) => "vs_lod_ssr",
        (false, true, false, _) => "vs_lod",
        (false, false, _, true) => "vs_wading",
        (false, false, _, false) => "vs_basic",
    }
}

fn fragment_entry_of(variant: Variant) -> &'static str {
    match (variant.underwater, variant.lod, variant.ssr, variant.depth) {
        (true, _, _, _) => "fs_underwater",
        (false, true, true, _) => "fs_lod_ssr",
        (false, true, false, _) => "fs_lod",
        (false, false, _, true) if variant.interior => "fs_main_interior",
        (false, false, _, true) => "fs_main",
        (false, false, _, false) => "fs_refraction_only",
    }
}

const KEEP_DESTINATION: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::Zero,
    dst_factor: wgpu::BlendFactor::One,
    operation: wgpu::BlendOperation::Add,
};

/// The main passes and most debug views write the finished colour with zero alpha; the
/// destination alpha is left alone.
const REPLACE_COLOUR: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent::REPLACE,
    alpha: KEEP_DESTINATION,
};

/// The depth-fog pass blends its colour over the scene by its alpha (`SrcAlpha`,
/// `OneMinusSrcAlpha`); its debug view composites it over black, so the fog amount the alpha
/// carries shows as brightness.
const OVER_BLACK: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::Zero,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: KEEP_DESTINATION,
};

fn debug_blend(output: WaterDebugOutput) -> wgpu::BlendState {
    match output {
        WaterDebugOutput::DepthFog => OVER_BLACK,
        _ => REPLACE_COLOUR,
    }
}

struct PipelineSpec<'a> {
    label: &'static str,
    layout: &'a wgpu::PipelineLayout,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
}

fn create_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    spec: &PipelineSpec,
) -> wgpu::RenderPipeline {
    let options = wgpu::PipelineCompilationOptions::default();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(spec.vertex_entry),
            buffers: &[Some(Vertex::desc())],
            compilation_options: options.clone(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(spec.fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: spec.blend,
                write_mask: wgpu::ColorWrites::COLOR,
            })],
            compilation_options: options,
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // Tested against the opaque depth, never written: the surface must not
        // feed the HiZ pyramid, and the depth-fog and mask passes hold the
        // buffer read-only.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;
    use glam::UVec2;
    use wgpu::util::DeviceExt;

    use super::*;
    use crate::assets::types::water_shader::water_flags;
    use crate::rendering::{
        gpu_test::test_device,
        scene_copy::SceneCopy,
        shaders::composed::water::POSITION_ENTRY_MARKER,
        water::{
            constants::WaterDrawConstants,
            displacement::{DISPLACEMENT_QUAD_DRAW_REGISTER, DisplacementGpu, QUAD_REGISTER},
            permutation::{PassContext, select},
            ssr::tests::harness::{
                depth_texture, extent, f16_bits_to_f32, f32_to_f16_bits, read_mip,
            },
        },
    };

    #[test]
    fn every_requested_entry_point_exists_in_the_composed_module() {
        use naga::ShaderStage::{Fragment, Vertex};
        let entries = shaders::composed::entry_points(&shaders::composed::water::WATER);
        let has = |name: &str, stage| entries.iter().any(|(n, s)| n == name && *s == stage);
        for variant in Variant::ALL {
            let (vertex, fragment) = (vertex_entry_of(variant), fragment_entry_of(variant));
            assert!(has(vertex, Vertex), "{vertex} for {variant:?}");
            assert!(has(fragment, Fragment), "{fragment} for {variant:?}");
        }
        assert!(has("vs_position", Vertex));
        for fragment in [
            "fs_depth_fog",
            "fs_mask_surface",
            "fs_mask_wading",
            "fs_surface_depth",
        ] {
            assert!(has(fragment, Fragment), "{fragment}");
        }
    }

    #[test]
    fn every_water_pipeline_validates_against_the_shader_layouts() {
        let Some((device, _queue)) = test_device("water pipeline validation") else {
            return;
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let scene = SceneCopy::new(&device, 4, 4);
        let layouts = WaterLayouts::new(&device);
        let mut pipelines = WaterPipelines::new(&device, scene.layout(), &layouts);
        for variant in Variant::ALL {
            let _ = pipelines.shading(variant);
        }
        for output in WaterDebugOutput::ALL {
            pipelines.ensure_debug(&device, output);
            for variant in Variant::ALL {
                let _ = pipelines.debug(output, variant);
            }
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }

    const SIZE: u32 = 4;

    /// A full-target quad at depth 0.5 and everything needed to draw it through a water
    /// pipeline on a headless device.
    struct QuadFixture {
        device: wgpu::Device,
        queue: wgpu::Queue,
        scene: SceneCopy,
        layouts: WaterLayouts,
        pipelines: WaterPipelines,
        vertices: wgpu::Buffer,
        indices: wgpu::Buffer,
    }

    impl QuadFixture {
        /// Per-draw constants that place the quad's vertices unchanged and leave every other
        /// register zero.
        fn identity_constants() -> WaterDrawConstants {
            let mut constants = WaterDrawConstants::zeroed();
            constants.vs_cb2[3] = glam::Vec4::X;
            constants.vs_cb2[4] = glam::Vec4::Y;
            constants.vs_cb2[5] = glam::Vec4::Z;
            constants.vs_cb2[6] = glam::Vec4::W;
            constants
        }

        fn new(test: &str) -> Option<Self> {
            let (device, queue) = test_device(test)?;
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let layouts = WaterLayouts::new(&device);
            let pipelines = WaterPipelines::new(&device, scene.layout(), &layouts);

            let corner = |x: f32, y: f32| Vertex {
                position: [x, y, 0.5],
                tex_coords: [0.0; 2],
                normal: [0.0, 0.0, 1.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
            };
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_quad_test_vertices"),
                contents: bytemuck::cast_slice(&[
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_quad_test_indices"),
                contents: bytemuck::cast_slice(&[0u32, 2, 1, 0, 3, 2]),
                usage: wgpu::BufferUsages::INDEX,
            });
            Some(Self {
                device,
                queue,
                scene,
                layouts,
                pipelines,
                vertices,
                indices,
            })
        }

        /// Draws the quad with the per-draw `constants` through `pipeline` into a `format`
        /// target over a depth buffer cleared to `scene_depth` (reverse-Z: larger is nearer)
        /// and returns the target's texel bytes. `bind` sets the groups the pipeline reads
        /// besides the draw constants.
        fn draw(
            &self,
            pipeline: &wgpu::RenderPipeline,
            format: wgpu::TextureFormat,
            scene_depth: f32,
            constants: &WaterDrawConstants,
            bind: impl FnOnce(&mut wgpu::RenderPass<'_>),
        ) -> Vec<u8> {
            let constants_buffer =
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("water_quad_test_constants"),
                        contents: bytemuck::bytes_of(constants),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
            let constants_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water_quad_test_constants_group"),
                layout: &self.layouts.draw,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &constants_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(DRAW_CONSTANTS_STRIDE),
                    }),
                }],
            });
            let texel_bytes = format.block_copy_size(None).expect("a colour format");
            let target = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("water_quad_test_target"),
                size: extent(UVec2::splat(SIZE)),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let depth = self
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("water_quad_test_depth"),
                    size: extent(UVec2::splat(SIZE)),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: Texture::DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water_quad_test_depth_clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(scene_depth),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("water_quad_test_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: None,
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                bind(&mut pass);
                pass.set_bind_group(DRAW_GROUP, &constants_group, &[0]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            }
            self.queue.submit(std::iter::once(encoder.finish()));
            read_mip(
                &self.device,
                &self.queue,
                &target,
                0,
                UVec2::splat(SIZE),
                texel_bytes,
            )
        }
    }

    /// Draws a full-target quad at depth 0.5 through the pipeline `select`
    /// picks, into a `format` target, over a depth buffer cleared to
    /// `scene_depth` (reverse-Z: larger is nearer), and returns the target's
    /// texel bytes.
    fn render_quad(
        select: impl Fn(&WaterPipelines) -> &wgpu::RenderPipeline,
        format: wgpu::TextureFormat,
        scene_depth: f32,
    ) -> Option<Vec<u8>> {
        let fixture = QuadFixture::new("water quad pass")?;
        Some(fixture.draw(
            select(&fixture.pipelines),
            format,
            scene_depth,
            &QuadFixture::identity_constants(),
            |_| {},
        ))
    }

    fn render_mask(tag: MaskTag, scene_depth: f32) -> Option<Vec<u8>> {
        render_quad(|pipelines| pipelines.mask(tag), MASK_FORMAT, scene_depth)
    }

    fn render_surface_depth(scene_depth: f32) -> Option<Vec<f32>> {
        let texels = render_quad(
            WaterPipelines::surface_depth,
            SURFACE_DEPTH_FORMAT,
            scene_depth,
        )?;
        Some(
            texels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|texel| f32::from_le_bytes(*texel))
                .collect(),
        )
    }

    #[test]
    fn a_visible_underwater_surface_writes_its_depth() {
        let Some(depths) = render_surface_depth(0.0) else {
            return;
        };
        assert!(
            depths.iter().all(|&depth| (depth - 0.5).abs() < 1e-6),
            "{depths:?}"
        );
    }

    #[test]
    fn an_underwater_surface_behind_the_opaque_scene_leaves_no_depth() {
        let Some(depths) = render_surface_depth(1.0) else {
            return;
        };
        assert!(depths.iter().all(|&depth| depth.abs() < 1e-6), "{depths:?}");
    }

    #[test]
    fn a_visible_surface_tags_its_pixels_with_its_mask_value() {
        for tag in [MaskTag::Surface, MaskTag::Wading] {
            let Some(texels) = render_mask(tag, 0.0) else {
                return;
            };
            assert!(
                texels.iter().all(|&texel| texel == tag.value()),
                "{tag:?}: {texels:?}"
            );
        }
    }

    #[test]
    fn a_surface_behind_the_opaque_scene_leaves_the_mask_clear() {
        let Some(texels) = render_mask(MaskTag::Surface, 1.0) else {
            return;
        };
        assert!(texels.iter().all(|&texel| texel == 0), "{texels:?}");
    }

    #[test]
    fn the_mask_follows_the_target_size() {
        let Some((device, queue)) = test_device("water mask resize") else {
            return;
        };
        let layouts = WaterLayouts::new(&device);
        let displacement = DisplacementGpu::new(&device);
        let mut frame = FrameBindings::new(&device, &queue, &layouts, 8, 8, displacement.views());
        let first = frame.mask_view().clone();
        frame.ensure_target_size(&device, &layouts, 8, 8);
        assert_eq!(frame.mask_view(), &first, "the same size keeps the mask");
        frame.ensure_target_size(&device, &layouts, 16, 8);
        assert_ne!(frame.mask_view(), &first, "a new size recreates it");
    }

    #[test]
    fn every_debug_entry_point_exists_in_the_composed_module() {
        use naga::ShaderStage::{Fragment, Vertex};
        let entries = shaders::composed::entry_points(&shaders::composed::water::WATER);
        let has = |name: &str, stage| entries.iter().any(|(n, s)| n == name && *s == stage);
        for output in WaterDebugOutput::ALL {
            assert!(has(output.fragment_entry(), Fragment), "{output:?}");
            for variant in Variant::ALL {
                let vertex = debug_vertex_entry(output, variant);
                assert!(
                    has(vertex, Vertex),
                    "{vertex} for {output:?} on {variant:?}"
                );
            }
        }
    }

    #[test]
    fn the_debug_pipelines_are_created_only_for_the_selected_output() {
        let Some(mut fixture) = QuadFixture::new("water debug pipeline creation") else {
            return;
        };
        assert!(fixture.pipelines.debug.is_empty());
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::Ssr);
        assert!(!fixture.pipelines.debug.is_empty());
        assert!(
            fixture
                .pipelines
                .debug
                .keys()
                .all(|(output, _)| *output == WaterDebugOutput::Ssr)
        );
        let created = fixture.pipelines.debug.len();
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::Ssr);
        assert_eq!(
            fixture.pipelines.debug.len(),
            created,
            "ensure is idempotent"
        );
    }

    /// The depth shader's variant, by its flags: the surface whose vertex stand-in places `v1`
    /// at the origin.
    fn depth_variant() -> Variant {
        let variant = select(water_flags::DEPTH, PassContext::default()).variant;
        assert!(variant.depth && !variant.lod && !variant.underwater);
        variant
    }

    /// `cb12[35]` of the frame constants: the camera position the quad uv is relative to.
    const CAMERA_REGISTER: usize = 35;

    fn rgba8_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texel: [u8; 4],
    ) -> wgpu::TextureView {
        device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("water_debug_test_texel"),
                    size: extent(UVec2::ONE),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &texel,
            )
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Copies a uniform `colour` scene into the fixture's scene copy.
    fn fill_scene_copy(fixture: &QuadFixture, colour: glam::Vec4) {
        let size = UVec2::splat(SIZE);
        let bytes: Vec<u8> = (0..SIZE * SIZE)
            .flat_map(|_| colour.to_array())
            .flat_map(|channel| f32_to_f16_bits(channel).to_le_bytes())
            .collect();
        let hdr = fixture.device.create_texture_with_data(
            &fixture.queue,
            &wgpu::TextureDescriptor {
                label: Some("water_debug_test_scene"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &bytes,
        );
        let depth = depth_texture(&fixture.device, &fixture.queue, size, &[0.0; 16]);
        let mut encoder = fixture
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        fixture.scene.encode(&mut encoder, &hdr, &depth);
        fixture.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Frame constants as a debug draw reads them: the texel size of the target and the
    /// displacement quad (centre xy, reciprocal size) seen from `camera`.
    fn debug_constants(quad: glam::Vec4, camera: glam::Vec4) -> WaterFrameConstants {
        let mut constants = WaterFrameConstants::zeroed();
        let texel = (SIZE as f32).recip();
        constants.cb0[0] = glam::Vec4::new(texel, texel, 1.0, 1.0);
        constants.cb12[QUAD_REGISTER] = quad;
        constants.cb12[CAMERA_REGISTER] = camera;
        constants
    }

    /// The frame bindings of a debug draw: both displacement targets are the one `field`
    /// texel, and the constants are `constants`.
    fn debug_frame(
        fixture: &QuadFixture,
        field: [u8; 4],
        constants: &WaterFrameConstants,
    ) -> FrameBindings {
        let field = rgba8_texture(&fixture.device, &fixture.queue, field);
        let frame = FrameBindings::new(
            &fixture.device,
            &fixture.queue,
            &fixture.layouts,
            SIZE,
            SIZE,
            &[field.clone(), field],
        );
        frame.write(&fixture.queue, constants);
        frame
    }

    /// The texels of the quad drawn with the per-draw `constants` as `output` over the scene
    /// copy and `frame`, reading the displacement field as a wading surface does.
    fn draw_debug_with(
        fixture: &QuadFixture,
        frame: &FrameBindings,
        output: WaterDebugOutput,
        constants: &WaterDrawConstants,
    ) -> Vec<glam::Vec4> {
        let bytes = fixture.draw(
            fixture.pipelines.debug(output, depth_variant()),
            HDR_FORMAT,
            0.0,
            constants,
            |pass| {
                pass.set_bind_group(SCENE_GROUP, fixture.scene.bind_group(), &[]);
                pass.set_bind_group(FRAME_GROUP, frame.wading_group(0), &[]);
            },
        );
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|texel| {
                let channel = |index: usize| {
                    f16_bits_to_f32(u16::from_le_bytes([texel[index * 2], texel[index * 2 + 1]]))
                };
                glam::Vec4::new(channel(0), channel(1), channel(2), channel(3))
            })
            .collect()
    }

    fn draw_debug(
        fixture: &QuadFixture,
        frame: &FrameBindings,
        output: WaterDebugOutput,
    ) -> Vec<glam::Vec4> {
        draw_debug_with(fixture, frame, output, &QuadFixture::identity_constants())
    }

    fn assert_all_close(texels: &[glam::Vec4], expected: glam::Vec4) {
        assert_eq!(texels.len(), (SIZE * SIZE) as usize);
        for texel in texels {
            assert!(
                texel.abs_diff_eq(expected, 2e-3),
                "expected {expected} everywhere, got {texels:?}"
            );
        }
    }

    const NEUTRAL_FIELD: [u8; 4] = [128; 4];
    const NO_QUAD: glam::Vec4 = glam::Vec4::ZERO;

    #[test]
    fn the_coverage_output_paints_every_covered_pixel_magenta_and_leaves_alpha() {
        let Some(mut fixture) = QuadFixture::new("water debug coverage") else {
            return;
        };
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::Coverage);
        let frame = debug_frame(
            &fixture,
            NEUTRAL_FIELD,
            &debug_constants(NO_QUAD, glam::Vec4::ZERO),
        );
        let texels = draw_debug(&fixture, &frame, WaterDebugOutput::Coverage);
        assert_all_close(&texels, glam::Vec4::new(1.0, 0.0, 1.0, 0.0));
    }

    #[test]
    fn the_scene_copy_output_shows_the_copied_scene_colour() {
        let Some(mut fixture) = QuadFixture::new("water debug scene copy") else {
            return;
        };
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::SceneCopy);
        fill_scene_copy(&fixture, glam::Vec4::new(0.25, 0.5, 0.75, 1.0));
        let frame = debug_frame(
            &fixture,
            NEUTRAL_FIELD,
            &debug_constants(NO_QUAD, glam::Vec4::ZERO),
        );
        let texels = draw_debug(&fixture, &frame, WaterDebugOutput::SceneCopy);
        assert_all_close(&texels, glam::Vec4::new(0.25, 0.5, 0.75, 0.0));
    }

    fn bind_sharp_tap(fixture: &QuadFixture, frame: &mut FrameBindings, sharp: [u8; 4]) {
        // The blurred tap differs from every sharp one, so only the sharp tap can match.
        let views = ReflectionViews {
            sharp: rgba8_texture(&fixture.device, &fixture.queue, sharp),
            blurred: rgba8_texture(&fixture.device, &fixture.queue, [0, 255, 0, 255]),
        };
        frame.bind_reflection(&fixture.device, &fixture.layouts, Some(views));
    }

    #[test]
    fn the_ssr_output_shows_the_sharp_tap_weighted_by_its_coverage() {
        let Some(mut fixture) = QuadFixture::new("water debug ssr") else {
            return;
        };
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::Ssr);
        let mut frame = debug_frame(
            &fixture,
            NEUTRAL_FIELD,
            &debug_constants(NO_QUAD, glam::Vec4::ZERO),
        );

        bind_sharp_tap(&fixture, &mut frame, [255, 0, 0, 128]);
        let half_covered = draw_debug(&fixture, &frame, WaterDebugOutput::Ssr);
        assert_all_close(&half_covered, glam::Vec4::new(128.0 / 255.0, 0.0, 0.0, 0.0));

        bind_sharp_tap(&fixture, &mut frame, [255, 255, 255, 0]);
        let missed = draw_debug(&fixture, &frame, WaterDebugOutput::Ssr);
        assert_all_close(&missed, glam::Vec4::ZERO);
    }

    #[test]
    fn the_displacement_output_colours_the_field_on_the_quad_and_nothing_beyond_it() {
        let Some(mut fixture) = QuadFixture::new("water debug displacement") else {
            return;
        };
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::Displacement);
        let quad = glam::Vec4::new(0.0, 0.0, 1.0 / 2048.0, 0.0);
        let on_quad = debug_constants(quad, glam::Vec4::ZERO);
        let beyond_quad = debug_constants(quad, glam::Vec4::new(4096.0, 0.0, 0.0, 0.0));

        let above_rest = debug_frame(&fixture, [192, 64, 128, 128], &on_quad);
        let texels = draw_debug(&fixture, &above_rest, WaterDebugOutput::Displacement);
        assert_all_close(
            &texels,
            glam::Vec4::new(
                (192.0 / 255.0 - 0.5) * 2.0,
                (0.5 - 64.0 / 255.0) * 2.0,
                0.0,
                0.0,
            ),
        );

        let below_rest = debug_frame(&fixture, [64, 128, 128, 128], &on_quad);
        let texels = draw_debug(&fixture, &below_rest, WaterDebugOutput::Displacement);
        assert_all_close(
            &texels,
            glam::Vec4::new(
                0.0,
                (128.0 / 255.0 - 0.5) * 2.0,
                (0.5 - 64.0 / 255.0) * 2.0,
                0.0,
            ),
        );

        let outside = debug_frame(&fixture, [192, 64, 128, 128], &beyond_quad);
        let texels = draw_debug(&fixture, &outside, WaterDebugOutput::Displacement);
        assert_all_close(&texels, glam::Vec4::ZERO);
    }

    /// Per-draw constants of a displacement quad draw (`ps_cb2[0]`): the marker, the quad
    /// centre relative to the camera and the reciprocal size of a quad four units wide.
    fn quad_draw_constants(centre: glam::Vec2) -> WaterDrawConstants {
        let mut constants = QuadFixture::identity_constants();
        constants.ps_cb2[DISPLACEMENT_QUAD_DRAW_REGISTER] =
            glam::Vec4::new(1.0, centre.x, centre.y, 0.25);
        constants
    }

    #[test]
    fn a_displacement_quad_draw_discards_the_pixels_beyond_its_quad_in_every_debug_output() {
        let Some(mut fixture) = QuadFixture::new("water debug quad discard") else {
            return;
        };
        let outputs = [
            WaterDebugOutput::Coverage,
            WaterDebugOutput::SceneCopy,
            WaterDebugOutput::Ssr,
            WaterDebugOutput::Displacement,
        ];
        for output in outputs {
            fixture.pipelines.ensure_debug(&fixture.device, output);
        }
        fill_scene_copy(&fixture, glam::Vec4::new(0.25, 0.5, 0.75, 1.0));
        let quad = glam::Vec4::new(0.0, 0.0, 1.0 / 2048.0, 0.0);
        let mut frame = debug_frame(
            &fixture,
            [192, 64, 128, 128],
            &debug_constants(quad, glam::Vec4::ZERO),
        );
        bind_sharp_tap(&fixture, &mut frame, [255, 0, 0, 255]);

        // The quad's pixels sit at the origin of the camera-relative world: a quad centred
        // there covers them, one centred two quad sizes away does not.
        let on_the_quad = quad_draw_constants(glam::Vec2::ZERO);
        let beyond_the_quad = quad_draw_constants(glam::Vec2::new(8.0, 0.0));
        for output in outputs {
            let covered = draw_debug_with(&fixture, &frame, output, &on_the_quad);
            assert!(
                covered.iter().all(|texel| *texel != glam::Vec4::ZERO),
                "{output:?} draws on the quad: {covered:?}"
            );
            let beyond = draw_debug_with(&fixture, &frame, output, &beyond_the_quad);
            assert!(
                beyond.iter().all(|texel| *texel == glam::Vec4::ZERO),
                "{output:?} leaves the target clear beyond the quad: {beyond:?}"
            );
        }
    }

    /// The quad drawn at clip depth `depth`, which the depth-fog stand-in writes as its alpha.
    fn constants_at_depth(depth: f32) -> WaterDrawConstants {
        let mut constants = QuadFixture::identity_constants();
        constants.vs_cb2[5] = glam::Vec4::new(0.0, 0.0, 0.0, depth);
        constants
    }

    #[test]
    fn the_depth_fog_output_weights_the_pass_colour_by_its_alpha_through_the_position_entry() {
        let Some(mut fixture) = QuadFixture::new("water debug depth fog") else {
            return;
        };
        fixture
            .pipelines
            .ensure_debug(&fixture.device, WaterDebugOutput::DepthFog);
        let frame = debug_frame(
            &fixture,
            NEUTRAL_FIELD,
            &debug_constants(NO_QUAD, glam::Vec4::ZERO),
        );
        // The stand-in writes (0.25, 0.5, 0.75) plus the marker only the position vertex entry
        // sets to the blue channel, with the fragment depth as its alpha. The pass composites
        // that over black, so each depth gives the colour times its own alpha.
        let colour = glam::Vec3::new(0.25, 0.5, 0.75 + POSITION_ENTRY_MARKER);
        let mut seen = Vec::new();
        for depth in [0.5, 0.25] {
            let texels = draw_debug_with(
                &fixture,
                &frame,
                WaterDebugOutput::DepthFog,
                &constants_at_depth(depth),
            );
            assert_all_close(&texels, (colour * depth).extend(0.0));
            seen.push(texels[0]);
        }
        assert_ne!(
            seen[0], seen[1],
            "the output follows the alpha, not just the colour"
        );
    }
}
