//! The cloud layers of the weather: the meshes of the cloud model, one for each layer, textured
//! with the layer's cloud texture, tinted, scrolling and alpha blended over the sky and
//! everything in it. Like the other layers they are drawn at infinite depth around the camera.
//! One uniform and one bind group serve each layer drawn; a bind group is built again only when
//! its position in the draw order shows another texture.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;

use super::assets::{AssetState, SkyAssetError, report};
use super::cloud_geometry::{CloudGeometry, CloudVertex};
use super::cloud_scroll::ScrollOffsets;
use super::layer::{self, LAYER_GROUP, LayerPipeline, WorldGroupLayouts};
use crate::rendering::lighting::time_of_day::unmeasured::CLOUD_HORIZON_HAZE_SINE;
use crate::rendering::shaders;
use crate::utils::finite_or_zero;

/// A layer that is less opaque than this changes no 8 bit colour.
const MIN_OPACITY: f32 = 1.0 / 255.0;

/// The most layers drawn in a frame; two weathers of sixteen layers each take 32.
pub(super) const MAX_DRAWS: usize = 64;

/// One cloud layer of one weather, as this frame draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudLayerDraw {
    /// The weather's cloud layer, which is also the position of its mesh in the cloud model.
    pub layer: u8,
    /// The texture as the database stores its path below the textures folder
    /// (`Sky\CloudsLower01_d.dds`); the case does not matter.
    pub texture: Arc<str>,
    /// The layer's colour at this time of day, linear, decoded from the blended stored colour.
    pub color: Vec3,
    /// 0 to 1: the layer's alpha at this time of day; while the weather changes, blended into the
    /// other weather's or weighted by the share of its own (see the weather frame).
    pub alpha: f32,
    /// The layer's X and Y speed from the weather, which is at most 0.1 either way.
    pub scroll_speed: Vec2,
}

/// What the clouds show this frame, as plain data: the renderer stays testable without the
/// lighting state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloudsInput<'a> {
    /// The layers to draw, the first at the back. While one weather turns into another this holds
    /// the layers of both, so a layer index can be there twice. The frame lends its layers.
    pub layers: Cow<'a, [CloudLayerDraw]>,
    /// 0 for none: how much a lightning flash brightens the clouds, which take `1 + flash` times
    /// their colour.
    pub flash: f32,
    /// Draws this layer alone, for the dev menu.
    pub solo: Option<u8>,
}

fn opacity(layer: &CloudLayerDraw) -> f32 {
    finite_or_zero(layer.alpha).clamp(0.0, 1.0)
}

/// A layer that shows and the part of the cloud model it draws.
#[derive(Debug)]
pub(super) struct PlannedDraw<'a> {
    pub layer: &'a CloudLayerDraw,
    pub range: Range<u32>,
}

/// What of an input is drawn, before any texture is loaded.
#[derive(Debug)]
pub(super) struct DrawPlan<'a> {
    /// In the order of the input, at most [`MAX_DRAWS`].
    pub draws: Vec<PlannedDraw<'a>>,
    /// The layers that show but have no mesh in the model, each once.
    pub without_mesh: Vec<u8>,
    /// Whether more than [`MAX_DRAWS`] layers show, so the last ones are left out.
    pub truncated: bool,
}

/// Drops the layers that cannot be seen (solo of another layer, no opacity) and gives the rest
/// the mesh of their layer index: `ranges` holds the index range of each mesh of the model.
pub(super) fn plan_draws<'a>(input: &'a CloudsInput<'_>, ranges: &[Range<u32>]) -> DrawPlan<'a> {
    let mut plan = DrawPlan {
        draws: Vec::new(),
        without_mesh: Vec::new(),
        truncated: false,
    };
    for layer in input.layers.iter() {
        let shows =
            input.solo.is_none_or(|solo| solo == layer.layer) && opacity(layer) >= MIN_OPACITY;
        if !shows {
            continue;
        }
        match ranges.get(usize::from(layer.layer)) {
            None if !plan.without_mesh.contains(&layer.layer) => {
                plan.without_mesh.push(layer.layer);
            }
            Some(range) if !range.is_empty() && plan.draws.len() < MAX_DRAWS => {
                plan.draws.push(PlannedDraw {
                    layer,
                    range: range.clone(),
                });
            }
            Some(range) if !range.is_empty() => plan.truncated = true,
            _ => {}
        }
    }
    plan
}

/// Mirrors `CloudUniform` in `sky_clouds.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct CloudUniform {
    /// rgb: the radiance of a fully lit texel, w: the opacity of the layer.
    pub radiance_opacity: [f32; 4],
    /// xy: the texture coordinate offset of the scroll, z: the sine of the elevation at which
    /// the horizon haze has cleared.
    pub offset_haze: [f32; 4],
}

impl CloudUniform {
    /// `radiance_scale` is the sky radiance scale the sky colours are multiplied by.
    pub(super) fn new(
        layer: &CloudLayerDraw,
        offset: Vec2,
        flash: f32,
        radiance_scale: f32,
    ) -> Self {
        let brightness =
            finite_or_zero(radiance_scale).max(0.0) * (1.0 + finite_or_zero(flash).max(0.0));
        let [red, green, blue] = layer
            .color
            .to_array()
            .map(|channel| finite_or_zero(channel).max(0.0) * brightness);
        Self {
            radiance_opacity: [red, green, blue, opacity(layer)],
            offset_haze: [offset.x, offset.y, CLOUD_HORIZON_HAZE_SINE, 0.0],
        }
    }
}

struct CloudMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    ranges: Vec<Range<u32>>,
}

impl CloudMesh {
    fn new(device: &wgpu::Device, geometry: &CloudGeometry) -> Self {
        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("night_sky_cloud_vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("night_sky_cloud_indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            ranges: geometry.ranges.clone(),
        }
    }
}

/// The textures of the layers by the path their weather names, loaded the first time a layer
/// shows. One that cannot be loaded is remembered, so the failure is neither retried nor logged
/// again.
#[derive(Default)]
struct TextureCache(HashMap<Arc<str>, Option<wgpu::TextureView>>);

impl TextureCache {
    fn view(
        &mut self,
        path: &Arc<str>,
        load: impl FnOnce(&str) -> Result<wgpu::TextureView, SkyAssetError>,
    ) -> Option<&wgpu::TextureView> {
        self.0
            .entry(Arc::clone(path))
            .or_insert_with(|| load(path).inspect_err(report).ok())
            .as_ref()
    }
}

struct BoundTexture {
    path: Arc<str>,
    bind_group: wgpu::BindGroup,
}

/// The uniform of a draw position and the texture it is bound to.
struct Slot {
    uniform: wgpu::Buffer,
    bound: Option<BoundTexture>,
}

/// One slot for each position in the frame's draw order, as the draws of a frame cannot share a
/// uniform: every write lands before the first of them runs.
struct DrawSlots {
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: Vec<Slot>,
}

impl DrawSlots {
    fn new(device: &wgpu::Device, bind_group_layout: wgpu::BindGroupLayout) -> Self {
        Self {
            bind_group_layout,
            sampler: layer::sampler(
                device,
                "night_sky_clouds_sampler",
                wgpu::AddressMode::Repeat,
            ),
            slots: Vec::new(),
        }
    }

    /// Writes `uniform` to slot `index` and binds the texture `path` there, unless the slot has
    /// it bound already. Returns the slot's bind group.
    fn fill(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        index: usize,
        texture: (&Arc<str>, &wgpu::TextureView),
        uniform: &CloudUniform,
    ) -> wgpu::BindGroup {
        while self.slots.len() <= index {
            self.slots.push(Slot {
                uniform: layer::uniform_buffer::<CloudUniform>(device, "night_sky_clouds_uniform"),
                bound: None,
            });
        }
        let slot = &mut self.slots[index];
        queue.write_buffer(&slot.uniform, 0, bytemuck::bytes_of(uniform));
        let (path, view) = texture;
        let bound = match slot.bound.take() {
            Some(bound) if bound.path == *path => bound,
            _ => BoundTexture {
                path: Arc::clone(path),
                bind_group: layer::bind_group(
                    device,
                    "night_sky_clouds_bind_group",
                    &self.bind_group_layout,
                    &slot.uniform,
                    view,
                    &self.sampler,
                ),
            },
        };
        let bind_group = bound.bind_group.clone();
        slot.bound = Some(bound);
        bind_group
    }
}

struct FrameDraw {
    bind_group: wgpu::BindGroup,
    range: Range<u32>,
}

pub(super) struct CloudLayer {
    pipeline: wgpu::RenderPipeline,
    slots: DrawSlots,
    mesh: Option<CloudMesh>,
    textures: TextureCache,
    scroll: ScrollOffsets,
    draws: Vec<FrameDraw>,
}

impl CloudLayer {
    pub(super) fn new(device: &wgpu::Device, world_groups: WorldGroupLayouts<'_>) -> Self {
        let bind_group_layout = layer::bind_group_layout(device, "night_sky_clouds_bgl");
        let pipeline = layer::create_pipeline(
            device,
            &LayerPipeline {
                label: "Night Sky Clouds Pipeline",
                shader: &shaders::SKY_CLOUDS,
                world_groups,
                layer_layout: &bind_group_layout,
                vertex_buffers: &[Some(CloudVertex::layout())],
                topology: wgpu::PrimitiveTopology::TriangleList,
                color_blend: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        );
        Self {
            pipeline,
            slots: DrawSlots::new(device, bind_group_layout),
            mesh: None,
            textures: TextureCache::default(),
            scroll: ScrollOffsets::new(),
            draws: Vec::new(),
        }
    }

    /// Makes `geometry` the cloud model. A failure is logged and leaves the clouds undrawn.
    pub(super) fn install_mesh(
        &mut self,
        device: &wgpu::Device,
        geometry: Result<CloudGeometry, SkyAssetError>,
    ) -> AssetState {
        self.draws.clear();
        match geometry {
            Ok(geometry) => {
                self.mesh = Some(CloudMesh::new(device, &geometry));
                AssetState::Loaded
            }
            Err(error) => {
                report(&error);
                self.mesh = None;
                AssetState::failed(&error)
            }
        }
    }

    /// Scrolls the layers on by `dt` world seconds and readies this frame's draws. `load_texture`
    /// loads the texture a layer names the first time that layer shows; a layer whose texture
    /// fails to load is not drawn. `radiance_scale` is the sky radiance scale the sky colours
    /// are multiplied by.
    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: &CloudsInput<'_>,
        radiance_scale: f32,
        dt: f32,
        mut load_texture: impl FnMut(&str) -> Result<wgpu::TextureView, SkyAssetError>,
    ) {
        self.scroll.advance(&input.layers, dt);
        self.draws.clear();
        let Some(mesh) = &self.mesh else { return };
        let plan = plan_draws(input, &mesh.ranges);
        for &layer in &plan.without_mesh {
            report_layer_without_mesh(layer, mesh.ranges.len());
        }
        if plan.truncated {
            report_draw_limit();
        }
        for planned in plan.draws {
            let layer = planned.layer;
            let Some(view) = self.textures.view(&layer.texture, &mut load_texture) else {
                continue;
            };
            let uniform = CloudUniform::new(
                layer,
                self.scroll.offset(layer.layer),
                input.flash,
                radiance_scale,
            );
            let bind_group = self.slots.fill(
                device,
                queue,
                self.draws.len(),
                (&layer.texture, view),
                &uniform,
            );
            self.draws.push(FrameDraw {
                bind_group,
                range: planned.range,
            });
        }
    }

    /// Draws the layers of the last [`Self::update`], the first at the back.
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(mesh) = self.mesh.as_ref().filter(|_| !self.draws.is_empty()) else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        for draw in &self.draws {
            pass.set_bind_group(LAYER_GROUP, &draw.bind_group, &[]);
            pass.draw_indexed(draw.range.clone(), 0, 0..1);
        }
    }
}

fn report_layer_without_mesh(layer: u8, mesh_count: usize) {
    crate::utils::warn_once(
        &format!("night-sky:cloud-layer-without-mesh:{layer}"),
        || {
            format!(
                "cloud layer {layer} has no mesh: the cloud model has {mesh_count}; the layer is not drawn"
            )
        },
    );
}

fn report_draw_limit() {
    crate::utils::warn_once("night-sky:cloud-draw-limit", || {
        format!("more than {MAX_DRAWS} cloud layers show; the rest are not drawn")
    });
}
