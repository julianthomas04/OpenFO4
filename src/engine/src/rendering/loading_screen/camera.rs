//! The viewer's camera on a loading screen's model: where the model stands and how dragging, the
//! pan keys and the slow spin it starts with move it. Pure, so every rule is unit-tested.
//!
//! The scene is in engine space (Y-up, one unit = 10 game units, see [`crate::bethesda_space`]).
//! The camera sits at the origin and looks along Bethesda north (+Y), which is engine -Z. The
//! model is placed by its transform: `translation * rotation * scale`. The viewer's yaw turns
//! about the vertical axis, around the model's own position or, with `around_origin`, around the
//! origin. Zoom `z` scales the model's distance from the camera by `1 / (1 + z)`. Panning moves
//! the model along the camera's plane, within a window around its placement.

use glam::{Mat4, Quat, Vec3};

use super::art::{LoadingArt, ModelTransform};
use super::motion::{PanKey, PanKeys, is_full_turn, spun_yaw, wrap_half_turns};
use super::tuning::{
    DEGREES_PER_PIXEL, MAX_STEP_SECONDS, PAN_HALF_WINDOWS_PER_SECOND, SPIN_DEGREES_PER_SECOND,
    ZOOM_PER_PIXEL,
};
use crate::bethesda_space::{position_to_engine, rotation_to_engine};

const NEAR: f32 = 0.1;
const FAR: f32 = 2000.0;
/// Zoom never reaches -1, where the distance would be infinite.
const MIN_ZOOM: f32 = -0.9;
/// Where a model stored at the origin is put instead, in Bethesda units straight ahead.
const FALLBACK_DISTANCE: f32 = 600.0;

/// The turn the model makes by itself, counted from where it stood when it began.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spin {
    start_deg: f32,
    travelled_deg: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtCamera {
    transform: ModelTransform,
    rotation_limits_deg: (f32, f32),
    zoom_limits: (f32, f32),
    no_rotation: bool,
    /// The turn the model starts at, within its limits.
    start_yaw_deg: f32,
    yaw_deg: f32,
    zoom: f32,
    /// How far the pan keys have moved the model right and up, in engine units.
    pan: (f32, f32),
    /// The model may be moved this far from its placement along each axis, in engine units.
    pan_limit: f32,
    pan_keys: PanKeys,
    /// Turning by itself until the viewer first drags; `None` once they have.
    spin: Option<Spin>,
}

impl ArtCamera {
    /// A camera for `art`, which the pan keys may move `pan_limit` engine units from its
    /// placement along each axis.
    pub fn new(art: &LoadingArt, pan_limit: f32) -> Self {
        let (rotation_min, rotation_max) = art.rotation_limits_deg;
        let rotation_limits_deg = ordered(f32::from(rotation_min), f32::from(rotation_max));
        let (zoom_min, zoom_max) = ordered(art.zoom_limits.0, art.zoom_limits.1);
        let zoom_limits = (zoom_min.max(MIN_ZOOM), zoom_max.max(MIN_ZOOM));
        let start_deg = if art.no_rotation {
            0.0
        } else {
            0.0_f32.clamp(rotation_limits_deg.0, rotation_limits_deg.1)
        };
        Self {
            transform: art.transform,
            rotation_limits_deg,
            zoom_limits,
            no_rotation: art.no_rotation,
            start_yaw_deg: start_deg,
            yaw_deg: start_deg,
            zoom: 0.0_f32.clamp(zoom_limits.0, zoom_limits.1),
            pan: (0.0, 0.0),
            pan_limit: pan_limit.max(0.0),
            pan_keys: PanKeys::default(),
            spin: (!art.no_rotation).then_some(Spin {
                start_deg,
                travelled_deg: 0.0,
            }),
        }
    }

    /// The viewer's turn from the transform's rotation, in degrees.
    pub fn yaw_degrees(&self) -> f32 {
        self.yaw_deg
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// How far the pan keys have moved the model right and up, in engine units.
    pub fn pan(&self) -> (f32, f32) {
        self.pan
    }

    /// Whether the model is still turning by itself.
    pub fn is_spinning(&self) -> bool {
        self.spin.is_some()
    }

    /// A drag of `dx_px` pixels to the right and `dy_px` down. The horizontal part turns the
    /// model's near side to the right, wrapping round when its limits span a full turn and
    /// stopping at them otherwise; the vertical part zooms in when it goes up. A drag ends the
    /// model's spin for good.
    pub fn drag(&mut self, dx_px: f32, dy_px: f32) {
        if !dx_px.is_finite() || !dy_px.is_finite() {
            return;
        }
        if dx_px.abs() > 0.0 || dy_px.abs() > 0.0 {
            self.spin = None;
        }
        self.turn(dx_px * DEGREES_PER_PIXEL);
        let (min, max) = self.zoom_limits;
        self.zoom = (self.zoom - dy_px * ZOOM_PER_PIXEL).clamp(min, max);
    }

    fn turn(&mut self, degrees: f32) {
        if self.no_rotation {
            return;
        }
        let limits = self.rotation_limits_deg;
        let yaw = self.yaw_deg + degrees;
        self.yaw_deg = if is_full_turn(limits) {
            wrap_half_turns(yaw)
        } else {
            yaw.clamp(limits.0, limits.1)
        };
    }

    /// Puts the model back where it starts, still: no spin, no zoom, no pan and no held key.
    /// Two cameras of the same art look the same after this, whatever came before.
    pub fn pin(&mut self) {
        let (zoom_min, zoom_max) = self.zoom_limits;
        self.spin = None;
        self.yaw_deg = self.start_yaw_deg;
        self.zoom = 0.0_f32.clamp(zoom_min, zoom_max);
        self.pan = (0.0, 0.0);
        self.release_pan_keys();
    }

    /// A pan key went down or up.
    pub fn set_pan_key(&mut self, key: PanKey, held: bool) {
        self.pan_keys.set(key, held);
    }

    /// Lets go of every held pan key.
    pub fn release_pan_keys(&mut self) {
        self.pan_keys = PanKeys::default();
    }

    /// Lets `seconds` pass: the spin turns the model on and the held pan keys move it within
    /// its window. A step that is not a positive number changes nothing; a long one counts as
    /// [`MAX_STEP_SECONDS`].
    pub fn tick(&mut self, seconds: f32) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let step = seconds.min(MAX_STEP_SECONDS);
        if let Some(spin) = &mut self.spin {
            spin.travelled_deg += SPIN_DEGREES_PER_SECOND * step;
            self.yaw_deg = spun_yaw(spin.start_deg, spin.travelled_deg, self.rotation_limits_deg);
        }
        let (right, up) = self.pan_keys.direction();
        let limit = self.pan_limit;
        let reach = limit * PAN_HALF_WINDOWS_PER_SECOND * step;
        self.pan = (
            (self.pan.0 + right * reach).clamp(-limit, limit),
            (self.pan.1 + up * reach).clamp(-limit, limit),
        );
    }

    /// The model's position in engine units at the current zoom, before panning.
    fn position(&self) -> Vec3 {
        let [x, y, z] = self.transform.position;
        let stored = position_to_engine(x, y, z);
        let placed = if stored.length_squared() > 0.0 {
            stored
        } else {
            position_to_engine(0.0, FALLBACK_DISTANCE, 0.0)
        };
        placed / (1.0 + self.zoom)
    }

    pub fn model_matrix(&self) -> Mat4 {
        let [rx, ry, rz] = self.transform.rotation_radians;
        let orientation = Mat4::from_quat(rotation_to_engine(rx, ry, rz))
            * Mat4::from_scale(Vec3::splat(self.transform.scale));
        let position = Mat4::from_translation(self.position());
        let yaw = Mat4::from_quat(Quat::from_rotation_y(self.yaw_deg.to_radians()));
        let pan = Mat4::from_translation(Vec3::new(self.pan.0, self.pan.1, 0.0));
        if self.transform.around_origin {
            pan * yaw * position * orientation
        } else {
            pan * position * yaw * orientation
        }
    }

    /// The model seen from the origin, looking along -Z, `vertical_fov_radians` high; the field
    /// is the stage's (see [`super::stage_framing`]) and stays vertical, so a wider window shows
    /// more at the sides.
    pub fn view_proj(&self, aspect: f32, vertical_fov_radians: f32) -> Mat4 {
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        perspective(vertical_fov_radians, aspect) * view
    }
}

/// The projection of the loading screen's scene, the model's and the stage's alike.
pub fn perspective(vertical_fov_radians: f32, aspect: f32) -> Mat4 {
    Mat4::perspective_rh(
        vertical_fov_radians,
        aspect.max(f32::MIN_POSITIVE),
        NEAR,
        FAR,
    )
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are the exact limits the camera clamps to"
)]
mod tests {
    use super::*;

    fn art() -> LoadingArt {
        LoadingArt {
            model_path: "model.glb".into(),
            asset_root: ".".into(),
            transform: ModelTransform {
                position: [0.0, 500.0, 0.0],
                rotation_radians: [0.0; 3],
                scale: 1.0,
                around_origin: false,
            },
            rotation_limits_deg: (-30, 45),
            zoom_limits: (-0.5, 1.5),
            no_rotation: false,
        }
    }

    const PAN_LIMIT: f32 = 4.0;

    /// The field of view of the game's own stage.
    fn fov() -> f32 {
        crate::rendering::loading_screen::stage_framing::test_support::vanilla_framing()
            .vertical_fov_radians
    }

    fn camera(edit: impl FnOnce(&mut LoadingArt)) -> ArtCamera {
        let mut art = art();
        edit(&mut art);
        ArtCamera::new(&art, PAN_LIMIT)
    }

    /// A camera that has been dragged a little, so its spin is over.
    fn still_camera(edit: impl FnOnce(&mut LoadingArt)) -> ArtCamera {
        let mut camera = camera(edit);
        camera.drag(1.0, 0.0);
        camera.drag(-1.0, 0.0);
        camera
    }

    fn origin_of(matrix: Mat4) -> Vec3 {
        matrix.transform_point3(Vec3::ZERO)
    }

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn pinning_puts_a_moved_camera_back_at_its_start_without_spin_or_held_keys() {
        let fresh = camera(|_| {});
        let mut moved = camera(|_| {});
        moved.tick(1.0);
        moved.drag(30.0, -40.0);
        moved.set_pan_key(PanKey::Right, true);
        moved.tick(0.1);

        moved.pin();
        moved.tick(1.0);

        assert!(!moved.is_spinning());
        assert_eq!(moved.yaw_degrees(), fresh.yaw_degrees());
        assert_eq!(moved.zoom(), fresh.zoom());
        assert_eq!(moved.pan(), (0.0, 0.0));
    }

    #[test]
    fn pinning_keeps_the_start_inside_the_rotation_limits() {
        let mut limited = camera(|art| art.rotation_limits_deg = (20, 45));
        limited.drag(100.0, 0.0);
        limited.pin();
        assert_eq!(limited.yaw_degrees(), 20.0);
    }

    #[test]
    fn a_horizontal_drag_turns_the_model_by_degrees_per_pixel() {
        let mut camera = camera(|_| {});
        camera.drag(40.0, 0.0);
        assert!((camera.yaw_degrees() - 40.0 * DEGREES_PER_PIXEL).abs() < 1e-4);
    }

    #[test]
    fn a_horizontal_drag_stops_at_the_rotation_limits() {
        let mut camera = camera(|_| {});
        camera.drag(100_000.0, 0.0);
        assert_eq!(camera.yaw_degrees(), 45.0);
        camera.drag(-100_000.0, 0.0);
        assert_eq!(camera.yaw_degrees(), -30.0);
    }

    #[test]
    fn reversed_rotation_limits_are_put_in_order() {
        let mut camera = camera(|art| art.rotation_limits_deg = (45, -30));
        camera.drag(-100_000.0, 0.0);
        assert_eq!(camera.yaw_degrees(), -30.0);
    }

    #[test]
    fn a_full_turn_range_wraps_instead_of_stopping() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 180));
        camera.drag(200.0 / DEGREES_PER_PIXEL, 0.0);
        assert!(
            (camera.yaw_degrees() + 160.0).abs() < 1e-3,
            "{}",
            camera.yaw_degrees()
        );
        camera.drag(-100.0 / DEGREES_PER_PIXEL, 0.0);
        assert!((camera.yaw_degrees() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn limits_that_cover_only_one_side_of_the_full_turn_still_clamp() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 90));
        camera.drag(100_000.0, 0.0);
        assert_eq!(camera.yaw_degrees(), 90.0);
        camera.drag(-100_000.0, 0.0);
        assert_eq!(camera.yaw_degrees(), -180.0);
    }

    #[test]
    fn a_model_starts_inside_its_rotation_limits() {
        let mut camera = camera(|art| art.rotation_limits_deg = (10, 45));
        assert_eq!(camera.yaw_degrees(), 10.0);
        camera.tick(0.5);
        assert!((10.0..=45.0).contains(&camera.yaw_degrees()));
        let camera = self::camera(|art| art.rotation_limits_deg = (-45, -10));
        assert_eq!(camera.yaw_degrees(), -10.0);
    }

    #[test]
    fn a_model_that_cannot_rotate_starts_at_its_placement_whatever_the_limits() {
        let camera = camera(|art| {
            art.no_rotation = true;
            art.rotation_limits_deg = (10, 45);
        });
        assert_eq!(camera.yaw_degrees(), 0.0);
    }

    #[test]
    fn no_rotation_ignores_horizontal_drags_but_not_vertical_ones() {
        let mut camera = camera(|art| art.no_rotation = true);
        camera.drag(500.0, -100.0);
        assert_eq!(camera.yaw_degrees(), 0.0);
        assert!(camera.zoom() > 0.0, "the zoom still answers");
    }

    #[test]
    fn input_that_is_not_a_number_changes_nothing() {
        let mut camera = camera(|_| {});
        camera.drag(f32::NAN, 1.0);
        camera.drag(1.0, f32::INFINITY);
        camera.tick(f32::NAN);
        assert_eq!((camera.yaw_degrees(), camera.zoom()), (0.0, 0.0));
        assert!(camera.is_spinning(), "such a drag is no drag");
    }

    #[test]
    fn dragging_up_zooms_in_and_down_zooms_out_by_zoom_per_pixel() {
        let mut camera = still_camera(|_| {});
        camera.drag(0.0, -100.0);
        assert!((camera.zoom() - 100.0 * ZOOM_PER_PIXEL).abs() < 1e-5);
        camera.drag(0.0, 140.0);
        assert!((camera.zoom() + 40.0 * ZOOM_PER_PIXEL).abs() < 1e-5);
    }

    #[test]
    fn a_vertical_drag_stays_within_the_zoom_limits() {
        let mut camera = camera(|_| {});
        camera.drag(0.0, -100_000.0);
        assert_eq!(camera.zoom(), 1.5);
        camera.drag(0.0, 100_000.0);
        assert_eq!(camera.zoom(), -0.5);
    }

    #[test]
    fn a_vertical_drag_does_not_turn_the_model() {
        let mut camera = still_camera(|_| {});
        camera.drag(0.0, -300.0);
        assert_eq!(camera.yaw_degrees(), 0.0);
    }

    #[test]
    fn zoom_scales_the_distance_by_one_over_one_plus_zoom() {
        let mut camera = camera(|_| {});
        let stored = origin_of(camera.model_matrix());
        camera.drag(0.0, -100.0);
        assert!((camera.zoom() - 0.5).abs() < 1e-6);
        assert_close(origin_of(camera.model_matrix()), stored / 1.5);
    }

    #[test]
    fn a_zoom_range_that_excludes_zero_starts_at_its_nearest_end() {
        assert_eq!(camera(|art| art.zoom_limits = (0.5, 2.0)).zoom(), 0.5);
        assert_eq!(
            camera(|art| art.zoom_limits = (-3.0, -2.0)).zoom(),
            MIN_ZOOM
        );
    }

    #[test]
    fn the_model_stands_ahead_of_the_camera_at_its_stored_position() {
        let camera = camera(|_| {});
        assert_close(origin_of(camera.model_matrix()), Vec3::new(0.0, 0.0, -50.0));
    }

    #[test]
    fn a_model_stored_at_the_origin_is_put_straight_ahead() {
        let camera = camera(|art| art.transform.position = [0.0; 3]);
        assert_close(origin_of(camera.model_matrix()), Vec3::new(0.0, 0.0, -60.0));
    }

    #[test]
    fn the_model_turns_about_its_own_position_by_default() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-90, 90));
        camera.drag(90.0 / DEGREES_PER_PIXEL, 0.0);
        let matrix = camera.model_matrix();
        assert_close(origin_of(matrix), Vec3::new(0.0, 0.0, -50.0));
        assert_close(matrix.transform_vector3(Vec3::Z), Vec3::X);
    }

    #[test]
    fn with_around_origin_the_model_swings_about_the_camera() {
        let mut camera = camera(|art| {
            art.rotation_limits_deg = (-90, 90);
            art.transform.around_origin = true;
        });
        camera.drag(90.0 / DEGREES_PER_PIXEL, 0.0);
        assert_close(origin_of(camera.model_matrix()), Vec3::new(-50.0, 0.0, 0.0));
    }

    #[test]
    fn the_transform_scale_and_rotation_apply_to_the_model() {
        let camera = camera(|art| {
            art.transform.scale = 2.0;
            art.transform.rotation_radians = [0.0, 0.0, std::f32::consts::FRAC_PI_2];
        });
        let turned = camera.model_matrix().transform_vector3(Vec3::X);
        assert!((turned.length() - 2.0).abs() < 1e-4);
        assert!(turned.x.abs() < 1e-4, "{turned:?}");
    }

    #[test]
    fn a_point_straight_ahead_projects_to_the_middle_of_the_screen() {
        let camera = camera(|_| {});
        let clip = camera.view_proj(16.0 / 9.0, fov()) * Vec3::new(0.0, 0.0, -50.0).extend(1.0);
        assert!(clip.w > 0.0);
        assert!((clip.x / clip.w).abs() < 1e-5 && (clip.y / clip.w).abs() < 1e-5);
    }

    fn pixel_of(camera: &ArtCamera, point: Vec3, size: (f32, f32)) -> (f32, f32) {
        let clip = camera.view_proj(size.0 / size.1, fov()) * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        (f32::midpoint(ndc.x, 1.0) * size.0, f32::midpoint(1.0, -ndc.y) * size.1)
    }

    #[test]
    fn a_point_at_the_top_edge_of_the_vertical_field_projects_to_the_top_of_the_screen() {
        let camera = camera(|_| {});
        let half_fov = fov() / 2.0;
        let point = Vec3::new(0.0, 50.0 * half_fov.tan(), -50.0);
        let (_, y) = pixel_of(&camera, point, (2560.0, 1440.0));
        assert!(y.abs() < 1e-2, "{y}");
    }

    #[test]
    fn the_t51_screen_puts_the_top_of_the_armour_near_the_top_right_where_vanilla_shows_it() {
        let camera = camera(|art| {
            art.transform.position = [40.0, 275.0, -105.0];
            art.transform.rotation_radians = [0.0, 0.0, 3.874_631];
            art.rotation_limits_deg = (-180, 180);
        });
        let armour_top = position_to_engine(0.0, 0.0, 150.0);
        let (x, y) = pixel_of(
            &camera,
            camera.model_matrix().transform_point3(armour_top),
            (2560.0, 1440.0),
        );
        let vanilla_silhouette_top = (1752.0, 131.0);
        assert!(
            (x - vanilla_silhouette_top.0).abs() < 60.0
                && (y - vanilla_silhouette_top.1).abs() < 40.0,
            "({x}, {y})"
        );
    }

    #[test]
    fn the_model_spins_by_itself_at_the_spin_speed_until_dragged() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 180));
        camera.tick(0.1);
        assert!((camera.yaw_degrees() - SPIN_DEGREES_PER_SECOND * 0.1).abs() < 1e-4);
        camera.tick(0.1);
        assert!((camera.yaw_degrees() - SPIN_DEGREES_PER_SECOND * 0.2).abs() < 1e-4);
        assert!(camera.is_spinning());
    }

    #[test]
    fn a_spin_with_a_full_turn_of_room_wraps_round() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 180));
        let seconds = 190.0 / SPIN_DEGREES_PER_SECOND;
        for _ in 0..(seconds / MAX_STEP_SECONDS).ceil() as u32 {
            camera.tick(MAX_STEP_SECONDS);
        }
        assert!(
            camera.yaw_degrees() < 0.0,
            "past 180 degrees it comes round to {}",
            camera.yaw_degrees()
        );
    }

    #[test]
    fn a_spin_with_narrower_limits_turns_back_at_each_end() {
        let mut camera = camera(|_| {});
        let steps_to_the_end = (45.0 / SPIN_DEGREES_PER_SECOND / 0.1).round() as u32;
        let mut highest = f32::MIN;
        for _ in 0..steps_to_the_end + 20 {
            camera.tick(0.1);
            highest = highest.max(camera.yaw_degrees());
        }
        assert!(
            (highest - 45.0).abs() < 1e-3,
            "it reached its limit: {highest}"
        );
        assert!(camera.yaw_degrees() < 45.0, "and then turned back");
        let mut lowest = f32::MAX;
        for _ in 0..400 {
            camera.tick(0.1);
            let yaw = camera.yaw_degrees();
            assert!(
                (-30.0..=45.0).contains(&yaw),
                "{yaw} stays within the limits"
            );
            lowest = lowest.min(yaw);
        }
        assert!((lowest + 30.0).abs() < 1e-3, "it swept down to {lowest}");
    }

    #[test]
    fn dragging_ends_the_spin_for_good() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 180));
        camera.tick(1.0);
        camera.drag(10.0, 0.0);
        let yaw = camera.yaw_degrees();
        assert!(!camera.is_spinning());
        camera.tick(1.0);
        assert_eq!(camera.yaw_degrees(), yaw, "it no longer turns by itself");
    }

    #[test]
    fn a_drag_that_moves_nowhere_does_not_end_the_spin() {
        let mut camera = camera(|_| {});
        camera.drag(0.0, 0.0);
        assert!(camera.is_spinning());
    }

    #[test]
    fn a_model_that_may_not_rotate_never_spins() {
        let mut camera = camera(|art| art.no_rotation = true);
        assert!(!camera.is_spinning());
        camera.tick(5.0);
        assert_eq!(camera.yaw_degrees(), 0.0);
    }

    #[test]
    fn a_long_stalled_step_counts_as_the_longest_step() {
        let mut stalled = camera(|art| art.rotation_limits_deg = (-180, 180));
        let mut capped = stalled.clone();
        stalled.tick(100.0);
        capped.tick(MAX_STEP_SECONDS);
        assert_eq!(stalled.yaw_degrees(), capped.yaw_degrees());
    }

    #[test]
    fn a_held_pan_key_moves_the_model_across_the_screen_at_the_pan_speed() {
        let mut camera = still_camera(|_| {});
        camera.set_pan_key(PanKey::Right, true);
        camera.tick(0.1);
        let (right, up) = camera.pan();
        assert!((right - PAN_LIMIT * PAN_HALF_WINDOWS_PER_SECOND * 0.1).abs() < 1e-5);
        assert_eq!(up, 0.0);
        assert_close(
            origin_of(camera.model_matrix()),
            Vec3::new(right, 0.0, -50.0),
        );
    }

    #[test]
    fn each_pan_key_moves_the_model_its_own_way() {
        for (key, sign) in [
            (PanKey::Right, (1.0, 0.0)),
            (PanKey::Left, (-1.0, 0.0)),
            (PanKey::Up, (0.0, 1.0)),
            (PanKey::Down, (0.0, -1.0)),
        ] {
            let mut camera = still_camera(|_| {});
            camera.set_pan_key(key, true);
            camera.tick(0.1);
            let (x, y) = camera.pan();
            assert!(x * sign.1 == 0.0 && y * sign.0 == 0.0, "{key:?}: {x} {y}");
            assert!(x * sign.0 + y * sign.1 > 0.0, "{key:?}: {x} {y}");
        }
    }

    #[test]
    fn the_pan_stops_at_the_edge_of_its_window_on_both_sides_of_both_axes() {
        let mut camera = still_camera(|_| {});
        for key in [PanKey::Right, PanKey::Up] {
            camera.set_pan_key(key, true);
        }
        for _ in 0..100 {
            camera.tick(MAX_STEP_SECONDS);
        }
        assert_eq!(camera.pan(), (PAN_LIMIT, PAN_LIMIT));
        for key in [PanKey::Right, PanKey::Up] {
            camera.set_pan_key(key, false);
        }
        for key in [PanKey::Left, PanKey::Down] {
            camera.set_pan_key(key, true);
        }
        for _ in 0..200 {
            camera.tick(MAX_STEP_SECONDS);
        }
        assert_eq!(camera.pan(), (-PAN_LIMIT, -PAN_LIMIT));
    }

    #[test]
    fn a_released_pan_key_leaves_the_model_where_it_is() {
        let mut camera = still_camera(|_| {});
        camera.set_pan_key(PanKey::Right, true);
        camera.tick(0.1);
        camera.set_pan_key(PanKey::Right, false);
        let moved = camera.pan();
        camera.tick(1.0);
        assert_eq!(camera.pan(), moved);
    }

    #[test]
    fn a_model_with_no_pan_window_cannot_be_moved() {
        let mut camera = ArtCamera::new(&art(), 0.0);
        camera.set_pan_key(PanKey::Right, true);
        camera.tick(1.0);
        assert_eq!(camera.pan(), (0.0, 0.0));
        let mut camera = ArtCamera::new(&art(), f32::NAN);
        camera.set_pan_key(PanKey::Up, true);
        camera.tick(1.0);
        assert_eq!(camera.pan(), (0.0, 0.0));
    }

    #[test]
    fn panning_moves_a_model_that_swings_about_the_camera_along_the_screen_too() {
        let mut camera = still_camera(|art| art.transform.around_origin = true);
        camera.set_pan_key(PanKey::Up, true);
        camera.tick(0.1);
        let (_, up) = camera.pan();
        assert_close(origin_of(camera.model_matrix()), Vec3::new(0.0, up, -50.0));
    }
}
