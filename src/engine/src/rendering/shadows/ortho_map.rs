//! Orthographic caster maps for callers that own their depth target, such as the rain occlusion
//! map. The map is culled and drawn with the shadow cull's pipelines and the caster pipelines,
//! but by a cull of its own with the one draw region it needs: the cascades and the lamp views
//! keep every one of their view slots and draw regions, and no overflow readback has to tell
//! the map from them.

use super::shadow_cull::{CullSlots, ShadowCull};
use super::shadow_maps::SHADOW_MAP_FORMAT;
use super::shadow_pipelines::ORTHO_MAP_VIEW_SLOT;
use super::views::{ShadowView, ShadowViewSet};
use super::{CasterSlots, ShadowRenderer, ShadowScene};
use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};

/// The format of a map target: the one the caster pipelines draw depth in.
pub const MAP_FORMAT: wgpu::TextureFormat = SHADOW_MAP_FORMAT;

/// The views the map's cull gives draw regions to: the map, as a cascade, and no lamp.
const MAP_SLOTS: CullSlots = CullSlots {
    cascades: 1,
    lamps: 0,
};

/// The draw region the map's cull gives its one view.
const MAP_REGION: usize = 0;
const _: () = assert!(MAP_REGION < MAP_SLOTS.cascades as usize);

/// A map to render: the view that fills it and the reverse-Z depth target it goes to.
pub struct OrthoMapRender<'a> {
    pub view: ShadowView,
    /// A [`MAP_FORMAT`] attachment; cleared to depth 0 before the casters are drawn.
    pub target: &'a wgpu::TextureView,
    /// The profiled span of the cull and the pass together.
    pub profiled_as: GpuPass,
}

/// The cull of the maps, running the pipelines of `cull`.
pub(super) fn new_cull(
    cull: &ShadowCull,
    device: &wgpu::Device,
    object_instances: &wgpu::Buffer,
    terrain_instances: &wgpu::Buffer,
) -> ShadowCull {
    cull.sibling(device, MAP_SLOTS, object_instances, terrain_instances)
}

impl ShadowRenderer {
    /// Culls and draws the static casters of `render.view` into its target: one compute pass and
    /// one depth pass, however many frames the map is kept for. Characters cast no shadow into
    /// the map. At most one map per submission, as the cull's uniforms are written ahead of it.
    pub fn encode_ortho_map(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        scene: &ShadowScene,
        render: &OrthoMapRender<'_>,
        profiler: &GpuProfiler,
    ) {
        self.pipelines
            .write_view(queue, ORTHO_MAP_VIEW_SLOT, &render.view);
        let views = ShadowViewSet {
            cascades: vec![Some(render.view)],
            lamps: Vec::new(),
        };
        self.ortho_cull.dispatch(
            encoder,
            queue,
            &views,
            scene.object_count,
            scene.terrain_count,
            profiler.compute_writes(render.profiled_as, SpanPart::First),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Ortho Map Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: render.target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: profiler.render_writes(render.profiled_as, SpanPart::Last),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        Self::bind_scene_groups(&mut pass, scene);
        let slots = CasterSlots {
            draws: self.ortho_cull.draws(),
            view: ORTHO_MAP_VIEW_SLOT,
            region: MAP_REGION,
        };
        self.draw_casters(&mut pass, scene, slots, None);
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::SQRT_2;

    use glam::{Mat4, UVec2, Vec2, Vec3, Vec4};

    use super::*;
    use crate::rendering::shadows::cascades::{Cascade, CascadeSet};
    use crate::rendering::shadows::shadow_cull::{CASTER_KINDS, CasterKind, ShadowDraws};
    use crate::rendering::shadows::test_rig::{ShadowRig, Slab, read_u32s, rig};
    use crate::rendering::shadows::views::{MAX_SHADOW_VIEWS, ViewCull};
    use crate::rendering::water::ssr::tests::harness::read_f32_mip;

    const SIZE: u32 = 32;
    const HALF: f32 = 16.0;
    const HEIGHT: f32 = 32.0;

    /// A view down on the square of `SIZE` units around `centre`, `HEIGHT` units tall: one
    /// texel to the unit, depth 1 at the top, without the cut-out casters.
    fn top_down(centre: Vec2) -> ShadowView {
        top_down_skipping_alpha(centre, true)
    }

    fn top_down_skipping_alpha(centre: Vec2, skip_alpha: bool) -> ShadowView {
        let eye = Vec3::new(centre.x, HEIGHT, centre.y);
        let view = Mat4::look_to_rh(eye, Vec3::NEG_Y, Vec3::NEG_Z);
        let projection = Mat4::orthographic_rh(-HALF, HALF, -HALF, HALF, HEIGHT, 0.0);
        ShadowView {
            view_proj: projection * view,
            cull: ViewCull::Ortho {
                sphere: Vec4::new(centre.x, HEIGHT / 2.0, centre.y, HALF * SQRT_2),
                to_light: Vec3::Y,
                texel_world: 1.0,
                min_texels: 0.0,
                skip_alpha,
            },
        }
    }

    fn map_texture(device: &wgpu::Device) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ortho_map_test_target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn encode_map(
        rig: &ShadowRig,
        encoder: &mut wgpu::CommandEncoder,
        view: ShadowView,
        target: &wgpu::Texture,
    ) {
        rig.renderer.encode_ortho_map(
            encoder,
            &rig.queue,
            &rig.world.scene(),
            &OrthoMapRender {
                view,
                target: &target.create_view(&wgpu::TextureViewDescriptor::default()),
                profiled_as: GpuPass::RainOcclusion,
            },
            &rig.profiler,
        );
    }

    fn draw_map(rig: &ShadowRig, view: ShadowView, target: &wgpu::Texture) -> Vec<f32> {
        let mut encoder = rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encode_map(rig, &mut encoder, view, target);
        rig.queue.submit(std::iter::once(encoder.finish()));
        read_f32_mip(&rig.device, &rig.queue, target, 0, UVec2::splat(SIZE))
    }

    /// The depth of the texel at (`x`, `z`) of the window `top_down(centre)` shows.
    fn depth_in(depths: &[f32], centre: Vec2, x: f32, z: f32) -> f32 {
        let column = (x - centre.x + HALF).floor() as usize;
        let row = (z - centre.y + HALF).floor() as usize;
        depths[row * SIZE as usize + column]
    }

    fn depth_at(depths: &[f32], x: f32, z: f32) -> f32 {
        depth_in(depths, Vec2::ZERO, x, z)
    }

    fn slab(x: f32, height: f32, z: f32, half_size: f32) -> Slab {
        Slab {
            centre: Vec3::new(x, height, z),
            half_size,
            alpha_tested: false,
        }
    }

    fn assert_depth(actual: f32, height: f32, what: &str) {
        let expected = height / HEIGHT;
        assert!(
            (actual - expected).abs() < 1e-5,
            "{what}: depth {actual}, expected {expected}"
        );
    }

    #[test]
    fn a_slab_is_drawn_at_the_depth_of_its_height_and_the_ground_around_it_stays_empty() {
        let Some(mut rig) = rig("a_slab_is_drawn_at_the_depth_of_its_height") else {
            return;
        };
        rig.place(&[slab(4.0, 10.0, -4.0, 3.0)], &[]);
        let target = map_texture(&rig.device);

        let depths = draw_map(&rig, top_down(Vec2::ZERO), &target);

        assert_depth(depth_at(&depths, 3.5, -4.5), 10.0, "under the slab");
        assert_eq!(depth_at(&depths, 7.5, -4.5), 0.0, "beside the slab");
        assert_eq!(depth_at(&depths, -9.5, 10.5), 0.0, "far from the slab");
    }

    #[test]
    fn the_higher_of_an_object_and_a_landscape_piece_is_what_a_texel_holds() {
        let Some(mut rig) = rig("the_higher_of_an_object_and_a_landscape_piece") else {
            return;
        };
        rig.place(&[slab(0.0, 20.0, 4.0, 4.0)], &[slab(-4.0, 8.0, 4.0, 4.0)]);
        let target = map_texture(&rig.device);

        let depths = draw_map(&rig, top_down(Vec2::ZERO), &target);

        assert_depth(
            depth_at(&depths, -2.5, 3.5),
            20.0,
            "object over the landscape",
        );
        assert_depth(depth_at(&depths, -6.5, 3.5), 8.0, "landscape alone");
        assert_depth(depth_at(&depths, 2.5, 3.5), 20.0, "object alone");
    }

    #[test]
    fn cut_out_casters_are_drawn_only_into_maps_whose_view_keeps_them() {
        let Some(mut rig) = rig("cut_out_casters_are_drawn_only_into_maps") else {
            return;
        };
        rig.place(
            &[Slab {
                alpha_tested: true,
                ..slab(4.0, 10.0, -4.0, 3.0)
            }],
            &[],
        );
        let target = map_texture(&rig.device);

        let skipped = draw_map(&rig, top_down(Vec2::ZERO), &target);
        let kept = draw_map(&rig, top_down_skipping_alpha(Vec2::ZERO, false), &target);

        assert_eq!(depth_at(&skipped, 3.5, -4.5), 0.0, "view skips cut-outs");
        assert_depth(depth_at(&kept, 3.5, -4.5), 10.0, "view keeps cut-outs");
    }

    #[test]
    fn drawing_a_map_again_forgets_what_the_target_held() {
        let Some(mut rig) = rig("drawing_a_map_again_forgets") else {
            return;
        };
        rig.place(&[slab(4.0, 10.0, -4.0, 3.0)], &[]);
        let target = map_texture(&rig.device);
        draw_map(&rig, top_down(Vec2::ZERO), &target);

        let depths = draw_map(&rig, top_down(Vec2::new(100.0, 0.0)), &target);

        assert!(depths.iter().all(|&depth| depth == 0.0));
    }

    #[test]
    fn a_map_is_drawn_from_draw_buffers_that_have_grown() {
        let Some(mut rig) = rig("a_map_is_drawn_from_draw_buffers_that_have_grown") else {
            return;
        };
        let grown = 20_000;
        let kinds = [CasterKind::Opaque, CasterKind::Terrain];
        let capacities =
            |draws: &ShadowDraws| kinds.map(|kind| draws.cascade_capacities()[kind.index()]);
        let before = capacities(rig.renderer.ortho_cull.draws());
        assert!(before.iter().all(|&capacity| capacity < grown));

        assert!(rig.renderer.resize_draws(&rig.device, [grown, 0, grown]));
        rig.place(&[slab(4.0, 10.0, -4.0, 3.0)], &[slab(-4.0, 8.0, 4.0, 3.0)]);

        for draws in [rig.renderer.ortho_cull.draws(), rig.renderer.cull.draws()] {
            assert!(capacities(draws).iter().all(|&capacity| capacity >= grown));
        }
        let target = map_texture(&rig.device);
        let depths = draw_map(&rig, top_down(Vec2::ZERO), &target);
        assert_depth(depth_at(&depths, 3.5, -4.5), 10.0, "object");
        assert_depth(depth_at(&depths, -4.5, 5.5), 8.0, "landscape piece");
    }

    #[test]
    fn a_map_leaves_the_draws_of_the_sun_cascades_in_its_submission_alone() {
        let Some(mut rig) = rig("a_map_leaves_the_draws_of_the_sun_cascades") else {
            return;
        };
        let far = Vec2::new(100.0, 0.0);
        rig.place(
            &[
                slab(4.0, 10.0, -4.0, 3.0),
                slab(-6.0, 14.0, 6.0, 2.0),
                slab(far.x, 12.0, far.y, 3.0),
            ],
            &[],
        );
        let sun = top_down(Vec2::ZERO);
        let ViewCull::Ortho { sphere, .. } = sun.cull else {
            panic!("top_down is an orthographic view");
        };
        let cascades = CascadeSet {
            cascades: vec![Cascade {
                view_proj: sun.view_proj,
                sphere,
                texel_world: 1.0,
                min_texels: 0.0,
                skip_alpha: false,
                refresh: true,
            }],
            to_sun: Vec3::Y,
        };
        rig.renderer.prepare(&rig.queue, Some(&cascades), &[]);
        let target = map_texture(&rig.device);
        let mut encoder = rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        rig.renderer
            .encode(&mut encoder, &rig.queue, &rig.world.scene(), &rig.profiler);
        encode_map(&rig, &mut encoder, top_down(far), &target);
        rig.queue.submit(std::iter::once(encoder.finish()));

        let opaque_hits = |draws: &ShadowDraws| {
            let counts = read_u32s(
                &rig.device,
                &rig.queue,
                &draws.counts,
                CASTER_KINDS * MAX_SHADOW_VIEWS,
            );
            counts[ShadowDraws::count_offset(CasterKind::Opaque, 0) as usize / 4]
        };
        assert_eq!(opaque_hits(rig.renderer.cull.draws()), 2, "the cascade");
        assert_eq!(opaque_hits(rig.renderer.ortho_cull.draws()), 1, "the map");
        let depths = read_f32_mip(&rig.device, &rig.queue, &target, 0, UVec2::splat(SIZE));
        assert_depth(
            depth_in(&depths, far, far.x + 0.5, far.y + 0.5),
            12.0,
            "far slab",
        );
    }
}
