use std::time::{Duration, Instant};

use glam::Vec3;

/// Demand of a texture slot nothing currently uses.
pub const NO_DEMAND: f32 = 0.0;
/// World units per UV unit of a mesh without usable UVs: stretched so far that
/// its textures are asked for at full detail.
pub const NO_UV_DENSITY: f32 = f32::INFINITY;
/// Demand that calls for the full-resolution texture whatever its size.
pub const FULL_DETAIL: f32 = f32::INFINITY;

/// Camera travel (engine units, about 8 m) that triggers a full demand
/// recompute; also the least lookahead.
pub const RECOMPUTE_DISTANCE: f32 = 6.0;
/// Seconds of camera travel the demand looks ahead, covering the latency of a
/// texture read and of the demand sweep.
const LOOKAHEAD_SECONDS: f32 = 1.0;
/// Caps the lookahead so a teleport does not read as a very fast camera.
const MAX_LOOKAHEAD: f32 = 150.0;
/// Window over which camera speed is measured.
const SPEED_WINDOW: Duration = Duration::from_millis(500);
/// Nearest distance a surface is assumed to be at, avoiding a division by zero.
const MIN_DISTANCE: f32 = 0.05;

/// The part of the camera that decides how many pixels a surface covers.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ScreenView {
    /// Vertical field of view in radians (the narrowest the camera takes).
    pub vertical_fov: f32,
    pub width: u32,
    pub height: u32,
}

impl Default for ScreenView {
    fn default() -> Self {
        Self {
            vertical_fov: 60f32.to_radians(),
            width: 1920,
            height: 1080,
        }
    }
}

/// Where texture demand is measured from, and how to turn a distance into
/// screen pixels. Demand is the largest number of screen pixels one UV unit of
/// the texture can cover (`pixels per UV`): the policy divides the texture's
/// size by it to get the mip the GPU will sample.
///
/// This is an estimate of what the view will need, not a bound: surfaces that
/// turn out to need more detail are caught by the GPU feedback (`feedback`).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DemandView {
    /// `None` inside an interior: no distance information, full detail.
    camera: Option<Vec3>,
    /// Pixels per world unit at distance 1.
    pixel_scale: f32,
    lookahead: f32,
}

impl Default for DemandView {
    fn default() -> Self {
        Self::new(None, ScreenView::default(), 0.0)
    }
}

impl DemandView {
    /// A surface at distance `d` facing the image plane covers
    /// `height / (2 d tan(fov/2))` pixels per world unit. Surfaces towards a
    /// screen corner cover up to `1 + tan_v^2 + tan_h^2` times as many; that
    /// is left to the GPU feedback, which sees them where they are.
    pub fn new(camera: Option<Vec3>, screen: ScreenView, lookahead: f32) -> Self {
        let tan_vertical = (screen.vertical_fov * 0.5).tan();
        let pixels_per_unit = screen.height as f32 / (2.0 * tan_vertical);
        Self {
            camera,
            pixel_scale: pixels_per_unit,
            lookahead,
        }
    }

    pub fn camera(&self) -> Option<Vec3> {
        self.camera
    }

    /// Whether demand measured from `previous` is out of date for this view:
    /// the camera travelled `RECOMPUTE_DISTANCE`, entered or left an interior,
    /// or the screen changed.
    #[allow(
        clippy::float_cmp,
        reason = "change detection: any difference in the pixel scale triggers a recompute"
    )]
    pub fn differs_from(&self, previous: &Self) -> bool {
        let camera_moved = match (self.camera, previous.camera) {
            (Some(now), Some(before)) => now.distance(before) > RECOMPUTE_DISTANCE,
            (None, None) => false,
            _ => true,
        };
        camera_moved || self.pixel_scale != previous.pixel_scale
    }

    /// Distance `pixels_per_uv` divides by for the box `min..max` (nearest
    /// point, less the lookahead); `None` inside an interior. Diagnostics only.
    pub fn effective_distance(&self, min: Vec3, max: Vec3) -> Option<f32> {
        let camera = self.camera?;
        Some((camera.distance(camera.clamp(min, max)) - self.lookahead).max(MIN_DISTANCE))
    }

    pub fn pixel_scale(&self) -> f32 {
        self.pixel_scale
    }

    pub fn lookahead(&self) -> f32 {
        self.lookahead
    }

    /// Demand a surface inside the box `min..max` places on a texture whose UV
    /// unit spans `world_per_uv` world units on it: measured at the nearest
    /// point of the box, advanced by the lookahead.
    pub fn pixels_per_uv(&self, min: Vec3, max: Vec3, world_per_uv: f32) -> f32 {
        let Some(camera) = self.camera else {
            return FULL_DETAIL;
        };
        let distance = camera.distance(camera.clamp(min, max));
        let pixels =
            self.pixel_scale * world_per_uv / (distance - self.lookahead).max(MIN_DISTANCE);
        if pixels.is_nan() { FULL_DETAIL } else { pixels }
    }
}

/// Camera speed over a sliding window, for the lookahead.
#[derive(Debug, Default)]
pub struct CameraMotion {
    anchor: Option<(Instant, Vec3)>,
    speed: f32,
}

impl CameraMotion {
    pub fn observe(&mut self, now: Instant, position: Vec3) {
        let Some((since, from)) = self.anchor else {
            self.anchor = Some((now, position));
            return;
        };
        let elapsed = now.duration_since(since);
        if elapsed >= SPEED_WINDOW {
            self.speed = position.distance(from) / elapsed.as_secs_f32();
            self.anchor = Some((now, position));
        }
    }

    /// Distance the camera is expected to cover while a texture read is in
    /// flight; never below `RECOMPUTE_DISTANCE`, the staleness of the demand.
    pub fn lookahead(&self) -> f32 {
        (self.speed * LOOKAHEAD_SECONDS).clamp(RECOMPUTE_DISTANCE, MAX_LOOKAHEAD)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;

    fn screen_90_degrees() -> ScreenView {
        ScreenView {
            vertical_fov: std::f32::consts::FRAC_PI_2,
            width: 1000,
            height: 1000,
        }
    }

    fn view_at(camera: Vec3, lookahead: f32) -> DemandView {
        DemandView::new(Some(camera), screen_90_degrees(), lookahead)
    }

    const BOX_MIN: Vec3 = Vec3::new(10.0, -1.0, -1.0);
    const BOX_MAX: Vec3 = Vec3::new(12.0, 1.0, 1.0);

    #[test]
    fn pixels_per_uv_falls_with_the_inverse_of_distance() {
        // 90 degree fov over 1000 px: 500 px per world unit at distance 1.
        let demand = view_at(Vec3::ZERO, 0.0).pixels_per_uv(BOX_MIN, BOX_MAX, 2.0);
        assert!((demand - 100.0).abs() < 1e-3, "{demand}");
    }

    #[test]
    fn pixels_per_uv_measures_to_the_nearest_point_of_the_box() {
        // Camera at (0, 5, 0); nearest box point (10, 1, 0): distance sqrt(100 + 16).
        let demand = view_at(Vec3::new(0.0, 5.0, 0.0), 0.0).pixels_per_uv(BOX_MIN, BOX_MAX, 1.0);
        assert!((demand - 500.0 / 116.0_f32.sqrt()).abs() < 1e-3, "{demand}");
    }

    #[test]
    fn pixels_per_uv_lookahead_advances_the_camera() {
        let demand = view_at(Vec3::ZERO, 6.0).pixels_per_uv(BOX_MIN, BOX_MAX, 1.0);
        assert!((demand - 125.0).abs() < 1e-3, "{demand}");
    }

    #[test]
    fn pixels_per_uv_stays_finite_when_the_lookahead_passes_the_surface() {
        let demand = view_at(Vec3::ZERO, 50.0).pixels_per_uv(BOX_MIN, BOX_MAX, 1.0);
        assert!((demand - 10_000.0).abs() < 1e-1, "{demand}");
    }

    #[test]
    fn pixels_per_uv_inside_the_box_uses_the_minimum_distance() {
        let demand = view_at(Vec3::new(11.0, 0.0, 0.0), 0.0).pixels_per_uv(BOX_MIN, BOX_MAX, 1.0);
        assert!((demand - 10_000.0).abs() < 1e-1, "{demand}");
    }

    #[test]
    fn pixels_per_uv_without_a_camera_demands_full_detail() {
        let view = DemandView::new(None, screen_90_degrees(), 0.0);
        assert_eq!(view.pixels_per_uv(BOX_MIN, BOX_MAX, 1.0), FULL_DETAIL);
    }

    #[test]
    fn pixels_per_uv_of_a_surface_without_uvs_is_full_detail() {
        let view = view_at(Vec3::new(11.0, 0.0, 0.0), 0.0);
        assert_eq!(
            view.pixels_per_uv(BOX_MIN, BOX_MAX, NO_UV_DENSITY),
            FULL_DETAIL
        );
    }

    #[test]
    fn pixels_per_uv_of_nan_input_is_full_detail() {
        let view = view_at(Vec3::ZERO, 0.0);
        assert_eq!(view.pixels_per_uv(BOX_MIN, BOX_MAX, f32::NAN), FULL_DETAIL);
    }

    #[test]
    fn effective_distance_is_none_in_an_interior() {
        let view = DemandView::new(None, screen_90_degrees(), 0.0);
        assert_eq!(view.effective_distance(BOX_MIN, BOX_MAX), None);
    }

    #[test]
    fn effective_distance_subtracts_lookahead_down_to_the_minimum() {
        assert_eq!(
            view_at(Vec3::ZERO, 4.0).effective_distance(BOX_MIN, BOX_MAX),
            Some(6.0)
        );
        assert_eq!(
            view_at(Vec3::ZERO, 99.0).effective_distance(BOX_MIN, BOX_MAX),
            Some(MIN_DISTANCE)
        );
    }

    #[test]
    fn differs_from_needs_more_than_the_recompute_distance_of_travel() {
        let before = view_at(Vec3::ZERO, 0.0);
        let at_limit = view_at(Vec3::new(RECOMPUTE_DISTANCE, 0.0, 0.0), 0.0);
        let past_limit = view_at(Vec3::new(RECOMPUTE_DISTANCE + 0.1, 0.0, 0.0), 0.0);
        assert!(!at_limit.differs_from(&before));
        assert!(past_limit.differs_from(&before));
    }

    #[test]
    fn differs_from_when_entering_or_leaving_an_interior() {
        let outside = view_at(Vec3::ZERO, 0.0);
        let inside = DemandView::new(None, screen_90_degrees(), 0.0);
        assert!(inside.differs_from(&outside));
        assert!(outside.differs_from(&inside));
        assert!(!inside.differs_from(&inside));
    }

    #[test]
    fn differs_from_when_the_screen_changes() {
        let before = view_at(Vec3::ZERO, 0.0);
        let taller = DemandView::new(
            Some(Vec3::ZERO),
            ScreenView {
                height: 1080,
                ..screen_90_degrees()
            },
            0.0,
        );
        assert!(taller.differs_from(&before));
    }

    #[test]
    fn camera_lookahead_has_a_floor_before_any_speed_is_measured() {
        assert_eq!(CameraMotion::default().lookahead(), RECOMPUTE_DISTANCE);
    }

    #[test]
    fn camera_speed_is_measured_over_the_window() {
        let start = Instant::now();
        let mut motion = CameraMotion::default();
        motion.observe(start, Vec3::ZERO);
        motion.observe(start + SPEED_WINDOW, Vec3::new(20.0, 0.0, 0.0));
        // 20 units in half a second = 40 units/s, one second of lookahead.
        assert!((motion.lookahead() - 40.0).abs() < 1e-4);
    }

    #[test]
    fn camera_speed_ignores_observations_inside_the_window() {
        let start = Instant::now();
        let mut motion = CameraMotion::default();
        motion.observe(start, Vec3::ZERO);
        motion.observe(start + SPEED_WINDOW / 2, Vec3::new(500.0, 0.0, 0.0));
        assert_eq!(motion.lookahead(), RECOMPUTE_DISTANCE);
    }

    #[test]
    fn camera_lookahead_is_capped_so_a_teleport_is_not_a_fast_camera() {
        let start = Instant::now();
        let mut motion = CameraMotion::default();
        motion.observe(start, Vec3::ZERO);
        motion.observe(start + SPEED_WINDOW, Vec3::new(100_000.0, 0.0, 0.0));
        assert_eq!(motion.lookahead(), MAX_LOOKAHEAD);
    }
}
