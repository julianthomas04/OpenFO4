pub mod cascade_cache;
pub mod cascades;
pub mod lamps;
pub mod overflow;
pub mod settings;
pub mod shadow_cull;
pub mod shadow_maps;
pub mod shadow_pipelines;
pub mod state;
pub mod views;

pub use cascades::CascadeSet;
pub use settings::ShadowConfig;
pub use state::ShadowState;

use glam::Vec4;

use crate::rendering::{
    device_loss::DeviceLossMonitor,
    frustum,
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    mega_buffer::MegaBuffer,
    readback_ring::ReadbackError,
    render_pipelines::world_pass_layout::WorldPassLayout,
    skinning::{
        bind_layout::SkinnedBindLayout,
        materials::SurfaceKind,
        scene::{SkinnedScene, Visibility},
    },
};
use lamps::atlas::Tile;
use lamps::atlas_preview::LampAtlasPreview;
use lamps::atlas_texture::LampAtlasTexture;
use lamps::cache::LampView;
pub use overflow::ShadowDrawStats;
use overflow::{LampOverflow, ShadowCountsReadback};
use shadow_cull::{CASTER_KINDS, CasterKind, ShadowCull, ShadowDraws};
use shadow_maps::ShadowMaps;
use shadow_pipelines::{INSTANCE_GROUP, MATERIAL_GROUP, ShadowPipelines};
use views::ShadowViewSet;

/// The world-pass resources the shadow passes draw from.
pub struct ShadowScene<'a> {
    pub material_bind_group: &'a wgpu::BindGroup,
    pub instance_bind_group: &'a wgpu::BindGroup,
    pub object_mega: &'a MegaBuffer,
    pub terrain_mega: &'a MegaBuffer,
    pub object_count: u32,
    pub terrain_count: u32,
    pub skinned: SkinnedScene<'a>,
}

/// GPU side of the shadows: one caster cull over every view of the frame,
/// one depth pass per sun cascade, then one pass over the lamp atlas tiles
/// that re-render this frame.
pub struct ShadowRenderer {
    maps: ShadowMaps,
    lamp_atlas: LampAtlasTexture,
    atlas_preview: LampAtlasPreview,
    pipelines: ShadowPipelines,
    cull: ShadowCull,
    cascade_slots: usize,
    views: ShadowViewSet,
    lamp_tiles: Vec<Tile>,
    overflow: ShadowCountsReadback,
    lamp_overflow_warned: bool,
    cascade_overflow_active: bool,
    rendered_cascades: u32,
}

impl ShadowRenderer {
    pub fn new(
        device: &wgpu::Device,
        maps: ShadowMaps,
        lamp_atlas: LampAtlasTexture,
        config: &ShadowConfig,
        world_pass_layout: &WorldPassLayout,
        skinned_layout: &SkinnedBindLayout,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) -> Self {
        Self {
            maps,
            atlas_preview: LampAtlasPreview::new(device, lamp_atlas.view()),
            lamp_atlas,
            pipelines: ShadowPipelines::new(device, world_pass_layout, skinned_layout),
            cull: ShadowCull::new(
                device,
                config.cascade_count as u32,
                object_instances,
                terrain_instances,
            ),
            cascade_slots: config.cascade_count,
            views: ShadowViewSet::default(),
            lamp_tiles: Vec::new(),
            overflow: ShadowCountsReadback::new(device, config.cascade_count),
            lamp_overflow_warned: false,
            cascade_overflow_active: false,
            rendered_cascades: 0,
        }
    }

    /// `None` skips every sun shadow pass this frame; receivers then must not
    /// sample the maps either.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        cascades: Option<&CascadeSet>,
        lamp_views: &[LampView],
    ) {
        self.views = ShadowViewSet {
            cascades: cascades.map(CascadeSet::views).unwrap_or_default(),
            lamps: lamp_views.iter().map(|lamp| lamp.view).collect(),
        };
        self.lamp_tiles = lamp_views.iter().map(|lamp| lamp.tile).collect();
        for (slot, view) in self.views.slotted(self.cascade_slots) {
            self.pipelines.write_view(queue, slot, view);
        }
    }

    /// The cascades rendered since the last call; lets the cascade cache keep
    /// only renders that were actually encoded.
    pub fn take_rendered_cascades(&mut self) -> u32 {
        std::mem::take(&mut self.rendered_cascades)
    }

    pub fn atlas_preview_view(&self) -> &wgpu::TextureView {
        self.atlas_preview.view()
    }

    pub fn encode_atlas_preview(&self, encoder: &mut wgpu::CommandEncoder) {
        self.atlas_preview.encode(encoder);
    }

    pub fn resize_draws(&mut self, device: &wgpu::Device, potentials: [u32; CASTER_KINDS]) -> bool {
        self.cull.resize_draws(device, potentials)
    }

    pub fn draw_stats(&self) -> &ShadowDrawStats {
        &self.overflow.latest
    }

    /// Lamp tiles to re-render, read back a few frames late. Grows the lamp
    /// regions to fit; the flag is true when a buffer was reallocated.
    pub fn poll_overflow(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<(LampOverflow, bool), ReadbackError> {
        let overflow = self.overflow.poll(device, device_loss)?;
        self.warn_cascade_overflow_episode();
        if !overflow.overflowed() {
            return Ok((overflow, false));
        }
        if !self.lamp_overflow_warned {
            self.lamp_overflow_warned = true;
            tracing::warn!(
                "lamp shadow draw region overflowed (needed {:?} draws per view); growing it and re-rendering the affected lamps",
                overflow.needed
            );
        }
        let grew = self.cull.grow_lamp_regions(device, overflow.needed);
        Ok((overflow, grew))
    }

    fn warn_cascade_overflow_episode(&mut self) {
        let latest = self.overflow.latest;
        if latest.cascade_overflow && !self.cascade_overflow_active {
            tracing::warn!(
                "cascade shadow draw region overflowed (draws per cascade and kind {:?}, capacities {:?}); the farthest casters are missing from the sun shadows",
                latest.cascade_draws,
                self.cull.draws().cascade_capacities()
            );
        }
        self.cascade_overflow_active = latest.cascade_overflow;
    }

    pub fn post_submit(&mut self) {
        self.overflow.post_submit();
    }

    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) {
        self.cull
            .rebind(device, object_instances, terrain_instances);
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        scene: &ShadowScene,
        profiler: &GpuProfiler,
    ) {
        if self.views.is_empty() {
            return;
        }
        {
            let _span = tracing::info_span!("shadow_cull").entered();
            self.cull.dispatch(
                encoder,
                queue,
                &self.views,
                scene.object_count,
                scene.terrain_count,
                profiler,
            );
        }
        self.overflow.encode_copy(
            encoder,
            self.cull.draws(),
            &self.lamp_tiles,
            self.views.rendered_cascades(),
        );
        let _span = tracing::info_span!("shadow_passes").entered();
        let cascade_count = self.views.cascades.len();
        for cascade in 0..cascade_count {
            if self.views.cascades[cascade].is_none() {
                continue;
            }
            self.rendered_cascades |= 1 << cascade;
            let timestamp_writes =
                profiler.render_writes(GpuPass::shadow_cascade(cascade), SpanPart::Whole);
            self.encode_cascade(encoder, scene, cascade, timestamp_writes);
        }
        if !self.lamp_tiles.is_empty() {
            let timestamp_writes = profiler.render_writes(GpuPass::LampShadows, SpanPart::Whole);
            self.encode_lamps(encoder, scene, timestamp_writes);
        }
    }

    fn encode_cascade(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &ShadowScene,
        cascade: usize,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Cascade Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.maps.layer_view(cascade),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        Self::bind_scene_groups(&mut pass, scene);
        let planes = self.views.cascades[cascade]
            .as_ref()
            .map(|view| frustum::clip_planes(&view.view_proj));
        self.draw_casters(&mut pass, scene, cascade, planes.as_ref());
    }

    /// Tiles are not cleared as a whole: the atlas keeps every lamp tile that
    /// does not re-render this frame.
    fn encode_lamps(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &ShadowScene,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lamp Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.lamp_atlas.view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        Self::bind_scene_groups(&mut pass, scene);
        for (lamp, &tile) in self.lamp_tiles.iter().enumerate() {
            self.lamp_atlas.begin_tile(&mut pass, tile);
            let planes = frustum::clip_planes(&self.views.lamps[lamp].view_proj);
            self.draw_casters(&mut pass, scene, self.cascade_slots + lamp, Some(&planes));
        }
    }

    /// Once per pass: wgpu-core merges every view of the bindless material
    /// group into the pass's usage scope on each `set_bind_group`, so rebinding
    /// it per lamp tile multiplies that cost by the tile count.
    fn bind_scene_groups(pass: &mut wgpu::RenderPass<'_>, scene: &ShadowScene) {
        pass.set_bind_group(MATERIAL_GROUP, scene.material_bind_group, &[]);
        pass.set_bind_group(INSTANCE_GROUP, scene.instance_bind_group, &[]);
    }

    fn draw_casters(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scene: &ShadowScene,
        slot: usize,
        view_planes: Option<&[Vec4; 6]>,
    ) {
        self.pipelines.bind_view(pass, slot);

        let draws = self.cull.draws();
        bind_mega(pass, scene.object_mega);
        for kind in [CasterKind::Opaque, CasterKind::AlphaTest] {
            self.draw_kind(pass, draws, kind, slot);
        }
        bind_mega(pass, scene.terrain_mega);
        self.draw_kind(pass, draws, CasterKind::Terrain, slot);
        if let Some(planes) = view_planes {
            self.draw_skinned_casters(pass, scene, planes);
        }
    }

    /// Characters are drawn directly, to every view whose frustum they touch;
    /// blended surfaces and effect shapes cast no shadow, as for static geometry. They displace
    /// the instance group, which the next view's static casters need back.
    fn draw_skinned_casters(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scene: &ShadowScene,
        planes: &[Vec4; 6],
    ) {
        let drew = scene
            .skinned
            .draw(pass, Visibility::Frustum(planes), |part| {
                match part.material.kind {
                    _ if part.material.is_effect => None,
                    SurfaceKind::Opaque => Some(self.pipelines.skinned_pipeline(false)),
                    SurfaceKind::AlphaTest => Some(self.pipelines.skinned_pipeline(true)),
                    SurfaceKind::Blend | SurfaceKind::Multiply => None,
                }
            });
        if drew {
            pass.set_bind_group(INSTANCE_GROUP, scene.instance_bind_group, &[]);
        }
    }

    fn draw_kind(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draws: &ShadowDraws,
        kind: CasterKind,
        slot: usize,
    ) {
        pass.set_pipeline(self.pipelines.pipeline(kind));
        pass.multi_draw_indexed_indirect_count(
            draws.buffer(kind),
            draws.draw_offset(kind, slot),
            &draws.counts,
            ShadowDraws::count_offset(kind, slot),
            draws.capacity(kind, slot),
        );
    }
}

fn bind_mega(pass: &mut wgpu::RenderPass<'_>, mega: &MegaBuffer) {
    pass.set_vertex_buffer(0, mega.vertex_buffer.slice(..));
    pass.set_index_buffer(mega.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
}
