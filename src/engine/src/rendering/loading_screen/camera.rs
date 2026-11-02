//! The viewer's camera on a loading screen's model: where the model stands and how dragging and
//! the wheel move it. Pure, so every rule is unit-tested.
//!
//! The scene is in engine space (Y-up, one unit = 10 game units, see [`crate::bethesda_space`]).
//! The camera sits at the origin and looks along Bethesda north (+Y), which is engine -Z. The
//! model is placed by its transform: `translation * rotation * scale`. The viewer's yaw turns
//! about the vertical axis, around the model's own position or, with `around_origin`, around the
//! origin. Zoom `z` scales the model's distance from the camera by `1 / (1 + z)`.

use glam::{Mat4, Quat, Vec3};

use super::art::{LoadingArt, ModelTransform};
use crate::bethesda_space::{position_to_engine, rotation_to_engine};

/// Inferred: the game's loading art framing is not documented.
pub const VERTICAL_FOV_DEGREES: f32 = 50.0;
const NEAR: f32 = 0.1;
const FAR: f32 = 2000.0;
/// Inferred: a drag across a 1000 px wide window turns the model by 350 degrees.
const DEGREES_PER_PIXEL: f32 = 0.35;
/// Inferred: ten wheel lines span the zoom range of a typical screen.
const ZOOM_PER_LINE: f32 = 0.1;
/// Zoom never reaches -1, where the distance would be infinite.
const MIN_ZOOM: f32 = -0.9;
/// Where a model stored at the origin is put instead, in Bethesda units straight ahead.
const FALLBACK_DISTANCE: f32 = 600.0;
const HALF_TURN_DEGREES: f32 = 180.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ArtCamera {
    transform: ModelTransform,
    rotation_limits_deg: (f32, f32),
    zoom_limits: (f32, f32),
    no_rotation: bool,
    yaw_deg: f32,
    zoom: f32,
}

impl ArtCamera {
    pub fn new(art: &LoadingArt) -> Self {
        let (rotation_min, rotation_max) = art.rotation_limits_deg;
        let rotation_limits_deg = ordered(f32::from(rotation_min), f32::from(rotation_max));
        let (zoom_min, zoom_max) = ordered(art.zoom_limits.0, art.zoom_limits.1);
        let zoom_limits = (zoom_min.max(MIN_ZOOM), zoom_max.max(MIN_ZOOM));
        Self {
            transform: art.transform,
            rotation_limits_deg,
            zoom_limits,
            no_rotation: art.no_rotation,
            yaw_deg: 0.0,
            zoom: 0.0_f32.clamp(zoom_limits.0, zoom_limits.1),
        }
    }

    /// The viewer's turn from the transform's rotation, in degrees.
    pub fn yaw_degrees(&self) -> f32 {
        self.yaw_deg
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// A horizontal drag of `dx_px` pixels to the right turns the model's near side to the
    /// right. Limits that span a full turn wrap around instead of stopping.
    pub fn drag(&mut self, dx_px: f32) {
        if self.no_rotation || !dx_px.is_finite() {
            return;
        }
        let (min, max) = self.rotation_limits_deg;
        let yaw = self.yaw_deg + dx_px * DEGREES_PER_PIXEL;
        self.yaw_deg = if min <= -HALF_TURN_DEGREES && max >= HALF_TURN_DEGREES {
            wrap_half_turns(yaw)
        } else {
            yaw.clamp(min, max)
        };
    }

    /// Scrolling `lines` forwards (positive) zooms in.
    pub fn wheel(&mut self, lines: f32) {
        if !lines.is_finite() {
            return;
        }
        let (min, max) = self.zoom_limits;
        self.zoom = (self.zoom + lines * ZOOM_PER_LINE).clamp(min, max);
    }

    /// The model's position in engine units at the current zoom.
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
        if self.transform.around_origin {
            yaw * position * orientation
        } else {
            position * yaw * orientation
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let projection = Mat4::perspective_rh(
            VERTICAL_FOV_DEGREES.to_radians(),
            aspect.max(f32::MIN_POSITIVE),
            NEAR,
            FAR,
        );
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        projection * view
    }
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// `degrees` brought into `[-180, 180)`.
fn wrap_half_turns(degrees: f32) -> f32 {
    (degrees + HALF_TURN_DEGREES).rem_euclid(2.0 * HALF_TURN_DEGREES) - HALF_TURN_DEGREES
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

    fn camera(edit: impl FnOnce(&mut LoadingArt)) -> ArtCamera {
        let mut art = art();
        edit(&mut art);
        ArtCamera::new(&art)
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
    fn a_drag_turns_the_model_by_degrees_per_pixel() {
        let mut camera = camera(|_| {});
        camera.drag(40.0);
        assert!((camera.yaw_degrees() - 40.0 * DEGREES_PER_PIXEL).abs() < 1e-4);
    }

    #[test]
    fn a_drag_stops_at_the_rotation_limits() {
        let mut camera = camera(|_| {});
        camera.drag(100_000.0);
        assert_eq!(camera.yaw_degrees(), 45.0);
        camera.drag(-100_000.0);
        assert_eq!(camera.yaw_degrees(), -30.0);
    }

    #[test]
    fn reversed_rotation_limits_are_put_in_order() {
        let mut camera = camera(|art| art.rotation_limits_deg = (45, -30));
        camera.drag(-100_000.0);
        assert_eq!(camera.yaw_degrees(), -30.0);
    }

    #[test]
    fn a_full_turn_range_wraps_instead_of_stopping() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 180));
        camera.drag(200.0 / DEGREES_PER_PIXEL);
        assert!(
            (camera.yaw_degrees() + 160.0).abs() < 1e-3,
            "{}",
            camera.yaw_degrees()
        );
        camera.drag(-100.0 / DEGREES_PER_PIXEL);
        assert!((camera.yaw_degrees() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn limits_that_cover_only_one_side_of_the_full_turn_still_clamp() {
        let mut camera = camera(|art| art.rotation_limits_deg = (-180, 90));
        camera.drag(100_000.0);
        assert_eq!(camera.yaw_degrees(), 90.0);
        camera.drag(-100_000.0);
        assert_eq!(camera.yaw_degrees(), -180.0);
    }

    #[test]
    fn no_rotation_ignores_drags() {
        let mut camera = camera(|art| art.no_rotation = true);
        camera.drag(500.0);
        assert_eq!(camera.yaw_degrees(), 0.0);
    }

    #[test]
    fn input_that_is_not_a_number_changes_nothing() {
        let mut camera = camera(|_| {});
        camera.drag(f32::NAN);
        camera.wheel(f32::INFINITY);
        assert_eq!((camera.yaw_degrees(), camera.zoom()), (0.0, 0.0));
    }

    #[test]
    fn the_wheel_stays_within_the_zoom_limits() {
        let mut camera = camera(|_| {});
        camera.wheel(1000.0);
        assert_eq!(camera.zoom(), 1.5);
        camera.wheel(-1000.0);
        assert_eq!(camera.zoom(), -0.5);
    }

    #[test]
    fn zoom_scales_the_distance_by_one_over_one_plus_zoom() {
        let mut camera = camera(|_| {});
        let stored = origin_of(camera.model_matrix());
        camera.wheel(5.0);
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
        camera.drag(90.0 / DEGREES_PER_PIXEL);
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
        camera.drag(90.0 / DEGREES_PER_PIXEL);
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
        let clip = camera.view_proj(16.0 / 9.0) * Vec3::new(0.0, 0.0, -50.0).extend(1.0);
        assert!(clip.w > 0.0);
        assert!((clip.x / clip.w).abs() < 1e-5 && (clip.y / clip.w).abs() < 1e-5);
    }
}
