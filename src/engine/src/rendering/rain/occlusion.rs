//! The rain occlusion map: the highest static caster of every spot around the camera, seen from
//! straight above. A raindrop or a splash looks its column up here to learn whether a roof
//! stands over it. The shadow cull and the caster pipelines draw the map (see
//! `shadows::ortho_map`), and only again once the camera has left the middle of it or the
//! streamer has loaded or dropped something.

use glam::{Mat4, Vec2, Vec3};

use crate::bethesda_space::ENGINE_UNITS_PER_METER;
use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler};
use crate::rendering::shadows::ortho_map::{MAP_FORMAT, OrthoMapRender};
use crate::rendering::shadows::views::{ShadowView, ViewCull};
use crate::rendering::shadows::{ShadowRenderer, ShadowScene};

pub const MAP_RESOLUTION: u32 = 512;
const HALF_EXTENT: f32 = 48.0 * ENGINE_UNITS_PER_METER;
const HALF_HEIGHT: f32 = 80.0 * ENGINE_UNITS_PER_METER;
/// Side of the square the map covers, in engine units.
pub const EXTENT: f32 = 2.0 * HALF_EXTENT;
/// The span of heights its depth covers, in engine units.
pub const HEIGHT_RANGE: f32 = 2.0 * HALF_HEIGHT;
const TEXEL: f32 = EXTENT / MAP_RESOLUTION as f32;
/// The map is drawn again once the camera strays further than this share of the half extent,
/// or of the half height, from its centre.
const REFRESH_FRACTION: f32 = 0.25;
/// Casters spanning fewer texels (about 0.75 m) are no roof.
const MIN_CASTER_TEXELS: f32 = 4.0;

fn snap_to_texel(value: f32) -> f32 {
    (value / TEXEL).round() * TEXEL
}

/// The box of world the map covers: the square of [`EXTENT`] around its centre, and the
/// [`HEIGHT_RANGE`] of heights centred on the centre's height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcclusionWindow {
    centre: Vec3,
}

impl OcclusionWindow {
    /// The window around `eye`. Its horizontal centre sits on the texel grid, so a redraw puts
    /// every roof edge on the texels it covered before.
    pub fn around(eye: Vec3) -> Self {
        Self {
            centre: Vec3::new(snap_to_texel(eye.x), eye.y, snap_to_texel(eye.z)),
        }
    }

    /// Whether `eye` is still in the middle of the window, where the map need not be redrawn.
    pub fn is_centred_on(&self, eye: Vec3) -> bool {
        let offset = (eye - self.centre).abs();
        offset.x <= HALF_EXTENT * REFRESH_FRACTION
            && offset.z <= HALF_EXTENT * REFRESH_FRACTION
            && offset.y <= HALF_HEIGHT * REFRESH_FRACTION
    }

    /// The world height depth 0 stands for; a texel that holds it has no caster.
    pub fn bottom(&self) -> f32 {
        self.centre.y - HALF_HEIGHT
    }

    /// The corner of the window with the smallest x and z.
    pub fn origin(&self) -> Vec2 {
        Vec2::new(self.centre.x, self.centre.z) - Vec2::splat(HALF_EXTENT)
    }

    /// Where the horizontal position `xz` falls on the map, 0 to 1 across each axis (outside
    /// that range beyond the window). Mirrored by `occlusion_uv` in `rain.wgsl`.
    pub fn uv(&self, xz: Vec2) -> Vec2 {
        (xz - self.origin()) / EXTENT
    }

    /// The height of the highest caster of a texel holding `depth`, `None` where nothing was
    /// drawn. Mirrored by `occlusion_height` in `rain.wgsl`.
    pub fn height_from_depth(&self, depth: f32) -> Option<f32> {
        (depth > 0.0).then(|| self.bottom() + depth * HEIGHT_RANGE)
    }

    /// The view that draws the window's casters into the map.
    pub fn shadow_view(&self) -> ShadowView {
        ShadowView {
            view_proj: self.view_proj(),
            cull: ViewCull::Ortho {
                sphere: self.centre.extend(HALF_EXTENT * std::f32::consts::SQRT_2),
                to_light: Vec3::Y,
                texel_world: TEXEL,
                min_texels: MIN_CASTER_TEXELS,
                skip_alpha: true,
            },
        }
    }

    /// Looks straight down from the top of the window, with +x to the right of the map and +z
    /// down it. Reverse-Z like the shadow maps: the top of the window is depth 1, its bottom 0.
    fn view_proj(&self) -> Mat4 {
        let eye = Vec3::new(self.centre.x, self.centre.y + HALF_HEIGHT, self.centre.z);
        let view = Mat4::look_to_rh(eye, Vec3::NEG_Y, Vec3::NEG_Z);
        let projection = Mat4::orthographic_rh(
            -HALF_EXTENT,
            HALF_EXTENT,
            -HALF_EXTENT,
            HALF_EXTENT,
            HEIGHT_RANGE,
            0.0,
        );
        projection * view
    }
}

/// What the map was last drawn for.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    window: OcclusionWindow,
    /// The streamer's loaded generation: the static world the map shows.
    generation: u64,
}

#[derive(Debug, Default)]
struct RefreshTracker {
    drawn: Option<Drawn>,
}

impl RefreshTracker {
    fn is_due(&self, eye: Vec3, generation: u64) -> bool {
        self.drawn
            .is_none_or(|drawn| drawn.generation != generation || !drawn.window.is_centred_on(eye))
    }

    fn commit(&mut self, window: OcclusionWindow, generation: u64) {
        self.drawn = Some(Drawn { window, generation });
    }

    fn window(&self) -> Option<OcclusionWindow> {
        self.drawn.map(|drawn| drawn.window)
    }
}

/// The map's texture and what it shows.
pub struct OcclusionMap {
    view: wgpu::TextureView,
    tracker: RefreshTracker,
}

impl OcclusionMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rain_occlusion_map"),
            size: wgpu::Extent3d {
                width: MAP_RESOLUTION,
                height: MAP_RESOLUTION,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("rain_occlusion_map_view"),
                ..Default::default()
            }),
            tracker: RefreshTracker::default(),
        }
    }

    /// The depth the shaders read, valid once [`Self::window`] is `Some`.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The window the map shows, `None` until it is first drawn.
    pub fn window(&self) -> Option<OcclusionWindow> {
        self.tracker.window()
    }

    /// Whether the frame has to call [`Self::render`] before anything reads the map: it was
    /// never drawn, the camera left the middle of its window, or the streamer's loaded
    /// `generation` moved on. The frame does not ask in interiors, where no weather is drawn.
    pub fn is_due(&self, eye: Vec3, generation: u64) -> bool {
        self.tracker.is_due(eye, generation)
    }

    /// Draws the static casters around `eye` into the map ahead of the passes that read it.
    pub fn render(
        &mut self,
        shadows: &ShadowRenderer,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        scene: &ShadowScene,
        eye: Vec3,
        generation: u64,
        profiler: &GpuProfiler,
    ) {
        let window = OcclusionWindow::around(eye);
        shadows.encode_ortho_map(
            encoder,
            queue,
            scene,
            &OrthoMapRender {
                view: window.shadow_view(),
                target: &self.view,
                profiled_as: GpuPass::RainOcclusion,
            },
            profiler,
        );
        self.tracker.commit(window, generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EYE: Vec3 = Vec3::new(1234.5, 300.0, -987.25);

    fn texel_count(distance: f32) -> f32 {
        distance / TEXEL
    }

    #[test]
    fn the_window_centre_sits_on_the_texel_grid_within_half_a_texel_of_the_eye() {
        let window = OcclusionWindow::around(EYE);
        let origin = window.origin() + Vec2::splat(HALF_EXTENT);
        for (centre, eye) in [(origin.x, EYE.x), (origin.y, EYE.z)] {
            assert!(
                (texel_count(centre) - texel_count(centre).round()).abs() < 1e-2,
                "centre {centre} is off the grid"
            );
            assert!((centre - eye).abs() <= TEXEL * 0.5 + 1e-3);
        }
    }

    #[test]
    fn the_window_keeps_the_eye_height_unsnapped() {
        assert_eq!(OcclusionWindow::around(EYE).bottom(), EYE.y - HALF_HEIGHT);
    }

    #[test]
    fn moving_within_a_texel_leaves_the_window_where_it_was() {
        let moved = EYE + Vec3::new(TEXEL * 0.2, 0.0, -TEXEL * 0.2);
        assert_eq!(
            OcclusionWindow::around(EYE).origin(),
            OcclusionWindow::around(moved).origin()
        );
    }

    #[test]
    fn the_eye_is_centred_until_it_strays_a_quarter_of_the_half_extent() {
        let window = OcclusionWindow::around(EYE);
        let strayed = HALF_EXTENT * REFRESH_FRACTION;
        let margin = TEXEL;
        for direction in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            assert!(window.is_centred_on(EYE + direction * (strayed - margin)));
            assert!(!window.is_centred_on(EYE + direction * (strayed + margin)));
        }
    }

    #[test]
    fn the_eye_is_centred_until_it_strays_a_quarter_of_the_half_height() {
        let window = OcclusionWindow::around(EYE);
        let strayed = HALF_HEIGHT * REFRESH_FRACTION;
        assert!(window.is_centred_on(EYE + Vec3::Y * (strayed - 1.0)));
        assert!(window.is_centred_on(EYE - Vec3::Y * (strayed - 1.0)));
        assert!(!window.is_centred_on(EYE + Vec3::Y * (strayed + 1.0)));
        assert!(!window.is_centred_on(EYE - Vec3::Y * (strayed + 1.0)));
    }

    #[test]
    fn the_corners_of_the_window_map_to_the_corners_of_the_map() {
        let window = OcclusionWindow::around(EYE);
        let origin = window.origin();
        assert_eq!(window.uv(origin), Vec2::ZERO);
        assert!(
            window
                .uv(origin + Vec2::splat(EXTENT))
                .abs_diff_eq(Vec2::ONE, 1e-6)
        );
        assert!(
            window
                .uv(origin + Vec2::splat(HALF_EXTENT))
                .abs_diff_eq(Vec2::splat(0.5), 1e-6)
        );
    }

    #[test]
    fn a_point_projects_to_the_map_position_of_its_uv_and_to_the_depth_of_its_height() {
        let window = OcclusionWindow::around(EYE);
        let view_proj = window.view_proj();
        for point in [
            EYE,
            EYE + Vec3::new(30.0, 40.0, -50.0),
            EYE + Vec3::new(-120.0, -200.0, 90.0),
            EYE + Vec3::new(5.0, 500.0, 300.0),
        ] {
            let clip = view_proj.project_point3(point);
            let uv = window.uv(Vec2::new(point.x, point.z));
            assert!((clip.x * 0.5 + 0.5 - uv.x).abs() < 1e-4, "u of {point}");
            assert!((0.5 - clip.y * 0.5 - uv.y).abs() < 1e-4, "v of {point}");
            let height = window.height_from_depth(clip.z).expect("above the bottom");
            assert!(
                (height - point.y).abs() < 1e-2,
                "height of {point}: {height}"
            );
        }
    }

    #[test]
    fn depth_grows_from_nothing_at_the_bottom_to_one_at_the_top_of_the_window() {
        let window = OcclusionWindow::around(EYE);
        let depth_at = |height: f32| {
            window
                .view_proj()
                .project_point3(Vec3::new(EYE.x, height, EYE.z))
                .z
        };
        assert!(depth_at(window.bottom()).abs() < 1e-5);
        assert!((depth_at(window.bottom() + HEIGHT_RANGE) - 1.0).abs() < 1e-5);
        assert!(depth_at(EYE.y - 10.0) < depth_at(EYE.y + 10.0));
    }

    #[test]
    fn a_texel_nothing_was_drawn_to_has_no_height() {
        let window = OcclusionWindow::around(EYE);
        assert_eq!(window.height_from_depth(0.0), None);
        assert_eq!(
            window.height_from_depth(1.0),
            Some(window.bottom() + HEIGHT_RANGE)
        );
        assert_eq!(
            window.height_from_depth(0.5),
            Some(window.bottom() + HEIGHT_RANGE * 0.5)
        );
    }

    #[test]
    fn the_cull_reaches_every_corner_of_the_window_from_above() {
        let ViewCull::Ortho {
            sphere,
            to_light,
            texel_world,
            skip_alpha,
            ..
        } = OcclusionWindow::around(EYE).shadow_view().cull
        else {
            panic!("the map is an orthographic view");
        };
        assert_eq!(to_light, Vec3::Y);
        assert_eq!(texel_world, TEXEL);
        assert!(skip_alpha, "foliage and fences are no roof");
        let corner = Vec2::splat(HALF_EXTENT).length();
        assert!(sphere.w >= corner, "radius {} < corner {corner}", sphere.w);
    }

    #[test]
    fn a_map_is_due_until_it_has_been_drawn() {
        assert!(RefreshTracker::default().is_due(EYE, 7));
    }

    #[test]
    fn a_drawn_map_is_not_due_while_the_eye_stays_centred_and_the_world_stands() {
        let mut tracker = RefreshTracker::default();
        tracker.commit(OcclusionWindow::around(EYE), 7);
        assert!(!tracker.is_due(EYE, 7));
        assert!(!tracker.is_due(EYE + Vec3::new(10.0, 3.0, -10.0), 7));
    }

    #[test]
    fn a_drawn_map_is_due_once_the_eye_leaves_the_middle_of_its_window() {
        let mut tracker = RefreshTracker::default();
        tracker.commit(OcclusionWindow::around(EYE), 7);
        assert!(tracker.is_due(EYE + Vec3::X * HALF_EXTENT, 7));
    }

    #[test]
    fn a_drawn_map_is_due_once_the_streamer_loaded_or_dropped_something() {
        let mut tracker = RefreshTracker::default();
        tracker.commit(OcclusionWindow::around(EYE), 7);
        assert!(tracker.is_due(EYE, 8));
    }

    #[test]
    fn a_new_draw_replaces_the_window_and_generation_of_the_old_one() {
        let mut tracker = RefreshTracker::default();
        tracker.commit(OcclusionWindow::around(EYE), 7);
        let moved = EYE + Vec3::X * HALF_EXTENT;
        tracker.commit(OcclusionWindow::around(moved), 8);
        assert!(!tracker.is_due(moved, 8));
        assert_eq!(tracker.window(), Some(OcclusionWindow::around(moved)));
    }

    mod gpu {
        use glam::UVec2;

        use super::*;
        use crate::rendering::shadows::test_rig::{ShadowRig, Slab, rig};
        use crate::rendering::water::ssr::tests::harness::read_f32_mip;

        const GENERATION: u64 = 7;
        /// Wider than the cull's smallest caster, and the caster of every test.
        const ROOF_HALF_SIZE: f32 = 14.0;
        const ROOF: Vec3 = Vec3::new(20.0, 70.0, -15.0);

        fn slab(offset: Vec3, half_size: f32) -> Slab {
            Slab {
                centre: EYE + offset,
                half_size,
                alpha_tested: false,
            }
        }

        fn render(rig: &ShadowRig, map: &mut OcclusionMap) -> Vec<f32> {
            let mut encoder = rig
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            map.render(
                &rig.renderer,
                &mut encoder,
                &rig.queue,
                &rig.world.scene(),
                EYE,
                GENERATION,
                &rig.profiler,
            );
            rig.queue.submit(std::iter::once(encoder.finish()));
            read_f32_mip(
                &rig.device,
                &rig.queue,
                map.view().texture(),
                0,
                UVec2::splat(MAP_RESOLUTION),
            )
        }

        /// The height the drawn map reports over the spot `offset` from the eye.
        fn height_over(map: &OcclusionMap, depths: &[f32], offset: Vec3) -> Option<f32> {
            let window = map.window().expect("the map was drawn");
            let world = EYE + offset;
            let texel = (window.uv(Vec2::new(world.x, world.z)) * MAP_RESOLUTION as f32).as_uvec2();
            window.height_from_depth(depths[(texel.y * MAP_RESOLUTION + texel.x) as usize])
        }

        fn assert_height(actual: Option<f32>, offset: f32, what: &str) {
            let expected = EYE.y + offset;
            assert!(
                actual.is_some_and(|height| (height - expected).abs() < 0.05),
                "{what}: {actual:?}, expected {expected}"
            );
        }

        #[test]
        fn a_roof_reads_back_as_its_height_over_the_landscape_and_open_ground_as_none() {
            let Some(mut rig) = rig("a_roof_reads_back_as_its_height") else {
                return;
            };
            rig.place(
                &[slab(ROOF, ROOF_HALF_SIZE)],
                &[slab(Vec3::new(-20.0, -15.0, 10.0), 80.0)],
            );
            let mut map = OcclusionMap::new(&rig.device);

            let depths = render(&rig, &mut map);

            assert_height(height_over(&map, &depths, ROOF), ROOF.y, "under the roof");
            assert_height(
                height_over(&map, &depths, Vec3::new(-60.0, 0.0, 40.0)),
                -15.0,
                "landscape in the open",
            );
            assert_eq!(
                height_over(&map, &depths, Vec3::new(200.0, 0.0, 200.0)),
                None,
                "no caster"
            );
        }

        #[test]
        fn a_cut_out_caster_such_as_foliage_is_no_roof() {
            let Some(mut rig) = rig("a_cut_out_caster_such_as_foliage") else {
                return;
            };
            rig.place(
                &[Slab {
                    alpha_tested: true,
                    ..slab(ROOF, ROOF_HALF_SIZE)
                }],
                &[],
            );
            let mut map = OcclusionMap::new(&rig.device);

            let depths = render(&rig, &mut map);

            assert_eq!(height_over(&map, &depths, ROOF), None);
        }

        #[test]
        fn a_caster_smaller_than_a_few_texels_is_no_roof() {
            let Some(mut rig) = rig("a_caster_smaller_than_a_few_texels") else {
                return;
            };
            let small = ROOF_HALF_SIZE * 0.1;
            rig.place(&[slab(ROOF, small), slab(-ROOF, ROOF_HALF_SIZE)], &[]);
            let mut map = OcclusionMap::new(&rig.device);

            let depths = render(&rig, &mut map);

            assert_eq!(height_over(&map, &depths, ROOF), None, "the small one");
            assert_height(height_over(&map, &depths, -ROOF), -ROOF.y, "the large one");
        }

        #[test]
        fn rendering_the_map_records_the_window_and_the_generation_it_drew() {
            let Some(rig) = rig("rendering_the_map_records_the_window") else {
                return;
            };
            let mut map = OcclusionMap::new(&rig.device);
            assert_eq!(map.window(), None);
            assert!(map.is_due(EYE, GENERATION));

            render(&rig, &mut map);

            assert_eq!(map.window(), Some(OcclusionWindow::around(EYE)));
            assert!(!map.is_due(EYE, GENERATION));
            assert!(map.is_due(EYE, GENERATION + 1));
        }
    }
}
