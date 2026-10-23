//! The perk chart's poster: the four textures of the install and the overlay drawn multiplied
//! over the chart.
//!
//! The poster images are ordinary egui textures. egui cannot blend by multiplication, so the two
//! overlay images live in a render pipeline of their own that egui runs as a paint callback.

use std::io;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use egui::{Pos2, Rect, TextureId, Vec2};
use egui_wgpu::{Callback, CallbackResources, CallbackTrait, ScreenDescriptor};

use super::world_map_texture::resolve_below;
use crate::assets::pipboy_art::PerkGridTexture;
use crate::assets::types::texture::TextureError;
use crate::rendering::egui_pass::EguiPass;
use crate::rendering::texture::load_dds_view;

const SHADER: &str = include_str!("../shaders/perk_multiply.wgsl");

#[derive(Debug, thiserror::Error)]
pub enum PerkGridError {
    #[error("perk grid texture '{key}' is not in the install: {source}")]
    NotFound {
        key: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("perk grid texture '{key}' is unusable: {source}")]
    Unusable {
        key: &'static str,
        #[source]
        source: TextureError,
    },
}

/// A registered poster image and its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridImage {
    pub texture: TextureId,
    pub size: Vec2,
}

/// The images the chart draws; the overlay images are only sized here, their pixels belong to the
/// multiply pipeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerkGridTextures {
    pub top: GridImage,
    pub bottom: GridImage,
    pub overlay_top_size: Vec2,
    pub overlay_bottom_size: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayLayer {
    Top,
    Bottom,
}

impl OverlayLayer {
    fn index(self) -> usize {
        match self {
            OverlayLayer::Top => 0,
            OverlayLayer::Bottom => 1,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    alpha: f32,
    target_is_srgb: f32,
    padding: [f32; 2],
}

struct OverlayBinding {
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
}

struct MultiplyResources {
    pipeline: wgpu::RenderPipeline,
    overlays: [OverlayBinding; 2],
    target_is_srgb: bool,
}

fn view_size(view: &wgpu::TextureView) -> Vec2 {
    let texture = view.texture();
    Vec2::new(texture.width() as f32, texture.height() as f32)
}

fn load_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    texture: PerkGridTexture,
) -> Result<wgpu::TextureView, PerkGridError> {
    let key = texture.key();
    let relative = resolve_below(Path::new(asset_root), key)
        .map_err(|source| PerkGridError::NotFound { key, source })?;
    load_dds_view(device, queue, asset_root, &relative)
        .map_err(|source| PerkGridError::Unusable { key, source })
}

/// Loads the four poster textures below `asset_root`, registers the poster pair with `pass` and
/// installs the multiply pipeline with the overlay pair into it.
pub fn load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut EguiPass,
    asset_root: &str,
    target_format: wgpu::TextureFormat,
) -> Result<PerkGridTextures, PerkGridError> {
    let top = load_view(device, queue, asset_root, PerkGridTexture::Top)?;
    let bottom = load_view(device, queue, asset_root, PerkGridTexture::Bottom)?;
    let overlay_top = load_view(device, queue, asset_root, PerkGridTexture::OverlayTop)?;
    let overlay_bottom = load_view(device, queue, asset_root, PerkGridTexture::OverlayBottom)?;
    let textures = PerkGridTextures {
        top: GridImage {
            texture: pass.register_mipmapped_texture(device, &top),
            size: view_size(&top),
        },
        bottom: GridImage {
            texture: pass.register_mipmapped_texture(device, &bottom),
            size: view_size(&bottom),
        },
        overlay_top_size: view_size(&overlay_top),
        overlay_bottom_size: view_size(&overlay_bottom),
    };
    pass.insert_callback_resource(MultiplyResources::new(
        device,
        target_format,
        [&overlay_top, &overlay_bottom],
    ));
    Ok(textures)
}

impl MultiplyResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        overlays: [&wgpu::TextureView; 2],
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("perk_multiply_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("perk_multiply_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let overlays = overlays.map(|view| {
            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("perk_multiply_params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("perk_multiply_bind_group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            OverlayBinding { bind_group, params }
        });
        Self {
            pipeline: create_pipeline(device, target_format, &layout),
            overlays,
            target_is_srgb: target_format.is_srgb(),
        }
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("perk_multiply"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("perk_multiply_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("perk_multiply_pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Dst,
                        dst_factor: wgpu::BlendFactor::Zero,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

struct MultiplyOverlay {
    layer: OverlayLayer,
    rect: Rect,
    uv: Rect,
    alpha: f32,
}

/// `uv`, which spans `rect`, moved to span `rect` with its edges rounded to whole pixels: the
/// renderer rounds a callback's viewport the same way, so the texture stays where the unrounded
/// rect would have put it.
fn snapped_uv(rect: Rect, uv: Rect, pixels_per_point: f32) -> Rect {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return uv;
    }
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;
    let snapped = Rect::from_min_max(
        Pos2::new(snap(rect.min.x), snap(rect.min.y)),
        Pos2::new(snap(rect.max.x), snap(rect.max.y)),
    );
    let uv_at = |point: Pos2| {
        Pos2::new(
            uv.min.x + (point.x - rect.min.x) / rect.width() * uv.width(),
            uv.min.y + (point.y - rect.min.y) / rect.height() * uv.height(),
        )
    };
    Rect::from_min_max(uv_at(snapped.min), uv_at(snapped.max))
}

impl CallbackTrait for MultiplyOverlay {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = resources
            .get::<MultiplyResources>()
            .expect("the perk grid loader installs the multiply resources before any overlay");
        let uv = snapped_uv(self.rect, self.uv, screen.pixels_per_point);
        let params = Params {
            uv_min: uv.min.into(),
            uv_max: uv.max.into(),
            alpha: self.alpha,
            target_is_srgb: f32::from(u8::from(resources.target_is_srgb)),
            padding: [0.0; 2],
        };
        queue.write_buffer(
            &resources.overlays[self.layer.index()].params,
            0,
            bytemuck::bytes_of(&params),
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let resources = resources
            .get::<MultiplyResources>()
            .expect("the perk grid loader installs the multiply resources before any overlay");
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.overlays[self.layer.index()].bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

/// A paint callback that multiplies the framebuffer inside `rect` (stage pixels) with the overlay
/// layer's part `uv` at `alpha`: `dst * (1 - a + a * overlay)` where `a` is the texel alpha times
/// `alpha`.
pub fn multiply_overlay(
    rect: Rect,
    layer: OverlayLayer,
    uv: Rect,
    alpha: f32,
) -> egui::PaintCallback {
    Callback::new_paint_callback(
        rect,
        MultiplyOverlay {
            layer,
            rect,
            uv,
            alpha,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uv_follows_the_edges_the_viewport_rounds_to() {
        let rect = Rect::from_min_max(Pos2::new(10.4, 20.0), Pos2::new(110.4, 120.0));
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

        let snapped = snapped_uv(rect, uv, 1.0);

        assert!((snapped.min.x - (-0.4 / 100.0)).abs() < 1e-6);
        assert!((snapped.max.x - (1.0 - 0.4 / 100.0)).abs() < 1e-6);
        assert_eq!((snapped.min.y, snapped.max.y), (0.0, 1.0));
    }

    #[test]
    fn a_rect_that_is_already_on_whole_pixels_keeps_its_uv() {
        let rect = Rect::from_min_max(Pos2::new(8.0, 4.0), Pos2::new(48.0, 44.0));
        let uv = Rect::from_min_max(Pos2::new(0.25, 0.5), Pos2::new(0.75, 1.0));

        assert_eq!(snapped_uv(rect, uv, 2.0), uv);
    }

    #[test]
    fn an_empty_rect_keeps_its_uv() {
        let rect = Rect::from_min_max(Pos2::new(5.0, 5.0), Pos2::new(5.0, 9.0));
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

        assert_eq!(snapped_uv(rect, uv, 1.0), uv);
    }

    #[test]
    fn the_multiply_shader_is_valid_and_has_both_entry_points() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("the shader validates");
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect();
        assert!(entries.contains(&("vs_main", naga::ShaderStage::Vertex)));
        assert!(entries.contains(&("fs_main", naga::ShaderStage::Fragment)));
    }

    const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    const TARGET_SIZE: u32 = 4;
    /// A row of a texture copy is padded to this many bytes.
    const ROW_ALIGNMENT: u32 = 256;

    fn texture(device: &wgpu::Device, queue: &wgpu::Queue, texel: [u8; 4]) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("perk_multiply_test_overlay"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &texel,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Multiplies a target of `format` cleared to `backdrop` (the clear colour, as the target
    /// takes it) with an overlay of `texel` at `alpha` and returns the red channel of the first
    /// pixel as the target stores it.
    fn multiplied_red(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        backdrop: f64,
        texel: [u8; 4],
        alpha: f32,
    ) -> u8 {
        let overlay = texture(device, queue, texel);
        let resources = MultiplyResources::new(device, format, [&overlay, &overlay]);
        let params = Params {
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            alpha,
            target_is_srgb: f32::from(u8::from(format.is_srgb())),
            padding: [0.0; 2],
        };
        queue.write_buffer(
            &resources.overlays[0].params,
            0,
            bytemuck::bytes_of(&params),
        );
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("perk_multiply_test_target"),
            size: wgpu::Extent3d {
                width: TARGET_SIZE,
                height: TARGET_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("perk_multiply_test_staging"),
            size: u64::from(ROW_ALIGNMENT * TARGET_SIZE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("perk_multiply_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: backdrop,
                            g: backdrop,
                            b: backdrop,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&resources.pipeline);
            pass.set_bind_group(0, &resources.overlays[0].bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_ALIGNMENT),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: TARGET_SIZE,
                height: TARGET_SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("receiver outlives the map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("multiply readback poll failed");
        receiver.recv().unwrap().expect("staging buffer maps");
        let mapped = staging
            .slice(..)
            .get_mapped_range()
            .expect("staging range is mapped");
        mapped[0]
    }

    #[test]
    fn the_overlay_multiplies_the_framebuffer_by_the_texel_scaled_by_its_alpha() {
        let Some((device, queue)) = crate::rendering::gpu_test::test_device("perk_multiply") else {
            return;
        };
        let near = |actual: u8, expected: f32| (f32::from(actual) - expected).abs() <= 2.0;

        let half_grey = multiplied_red(
            &device,
            &queue,
            TARGET_FORMAT,
            0.8,
            [128, 128, 128, 255],
            1.0,
        );
        let half_covered = multiplied_red(
            &device,
            &queue,
            TARGET_FORMAT,
            0.8,
            [128, 128, 128, 255],
            0.5,
        );
        let transparent = multiplied_red(&device, &queue, TARGET_FORMAT, 0.8, [0, 0, 0, 0], 1.0);

        assert!(near(half_grey, 204.0 * 128.0 / 255.0), "{half_grey}");
        assert!(
            near(half_covered, 204.0 * (0.5 + 0.5 * 128.0 / 255.0)),
            "{half_covered}"
        );
        assert!(
            near(transparent, 204.0),
            "a transparent texel leaves the framebuffer: {transparent}"
        );
    }

    #[test]
    fn on_an_srgb_target_the_overlay_multiplies_the_stored_values_like_a_plain_target() {
        let Some((device, queue)) = crate::rendering::gpu_test::test_device("perk_multiply_srgb")
        else {
            return;
        };
        let linear = |encoded: f64| ((encoded + 0.055) / 1.055).powf(2.4);

        let stored = multiplied_red(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            linear(0.8),
            [128, 128, 128, 255],
            1.0,
        );

        let expected = 0.8 * 128.0;
        assert!(
            (f32::from(stored) - expected as f32).abs() <= 3.0,
            "{stored} is not near {expected}"
        );
    }

    /// Developer check against a real install: the four poster textures are found below the asset
    /// root and load as images of the width the grid scales them from.
    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installed perk grid textures and a GPU"]
    fn the_installed_poster_textures_load() {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let asset_root = format!("{install}/{}", crate::assets::layout::ASSETS_DIR);
        let Some((device, queue)) = crate::rendering::gpu_test::test_device("perk_grid_textures")
        else {
            return;
        };
        for texture in PerkGridTexture::ALL {
            let view = load_view(&device, &queue, &asset_root, texture)
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(view_size(&view), Vec2::splat(2048.0), "{}", texture.key());
        }
    }

    #[test]
    fn the_params_block_matches_the_shaders_uniform_layout() {
        assert_eq!(std::mem::size_of::<Params>(), 32);
    }
}
