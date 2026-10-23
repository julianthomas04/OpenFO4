//! The registers of the SSR chain: the camera facts it needs, the ray-setup
//! copy of the water frame constants, the raytracer's `cb0` and the Hi-Z
//! linearisation constants.
//!
//! The translated shaders expect a forward-Z view space (z grows away from the camera, 0 at
//! the near plane in NDC); this engine renders reverse-Z with an infinite far
//! plane. The registers here express the engine's camera in that convention: `cb12[0..2]` is the view rotation
//! (rows right, up, forward in Bethesda axes), `cb12[4..7]` the view-space to
//! clip projection with a forward-Z z row, and the linear depth constants
//! `(A, B, C) = (near, 1, MIN_RAW_DEPTH)` invert the reverse-Z depth (the sky,
//! raw depth 0, becomes a very large finite distance instead of a division by
//! zero).

use glam::{Mat3, Mat4, UVec2, Vec3, Vec4};

use crate::bethesda_space::{UNITS_FROM_ENGINE, axes_from_engine};
use crate::rendering::{
    camera::Camera,
    lighting::frame_uniform::FrameUniform,
    water::constants::{WaterFrameConstants, camera_relative_bethesda, frame_constants},
};

/// First `cb12` register of the view rotation rows.
const CB12_VIEW_ROWS: usize = 0;
/// First `cb12` register of the view-space to clip rows.
const CB12_PROJECTION_ROWS: usize = 4;

/// The camera of the frame as the SSR chain reads it, in engine space and units.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SsrCamera {
    forward: Vec3,
    up: Vec3,
    z_near: f32,
    far_distance: f32,
}

/// What surrounds the camera, which decides the far plane of Fallout 4's world
/// camera.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Surroundings {
    Exterior,
    /// An interior cell and its resolved fog clip distance (cell `XCLL`, or the
    /// lighting template's when the cell inherits it), in Bethesda units, when
    /// it has a positive one.
    Interior {
        clip_distance: Option<f32>,
    },
}

/// The smallest raw depth treated as finite: raw depth 0 (the sky) maps to a huge finite distance.
const MIN_RAW_DEPTH: f32 = 1e-9;

/// The far plane of an exterior and the ceiling of an interior's.
const MAX_FAR_DISTANCE: f32 = 353_840.0;

/// The far plane Fallout 4 gives its world camera, which is what the water SSR
/// reads: 353840 outdoors; in an interior the cell's fog clip distance (`XCLL`,
/// or the lighting template's when the inherit flag `0x80` is set) capped at
/// 353840 when positive, else 353840.
pub fn world_camera_far(surroundings: Surroundings) -> f32 {
    match surroundings {
        Surroundings::Exterior => MAX_FAR_DISTANCE,
        Surroundings::Interior { clip_distance } => clip_distance
            .filter(|distance| *distance > 0.0)
            .map_or(MAX_FAR_DISTANCE, |distance| distance.min(MAX_FAR_DISTANCE)),
    }
}

impl SsrCamera {
    /// The camera of a frame outdoors; [`Self::with_far`] sets another far plane.
    pub fn of(camera: &Camera) -> Self {
        Self {
            forward: camera.forward(),
            up: camera.up,
            z_near: camera.znear,
            far_distance: world_camera_far(Surroundings::Exterior),
        }
    }

    #[must_use]
    pub fn with_far(self, far_distance: f32) -> Self {
        Self {
            far_distance,
            ..self
        }
    }

    /// Near plane distance in Bethesda units (`cb0[0].z` of the raytracer).
    pub fn near(&self) -> f32 {
        self.z_near * UNITS_FROM_ENGINE
    }

    /// Far plane distance in Bethesda units (`cb0[0].w`): the frustum far of
    /// the world camera ([`world_camera_far`]); the engine's own projection has
    /// none. It bounds the ray and scales the depth-gap fade.
    pub fn far(&self) -> f32 {
        self.far_distance
    }

    /// The view rotation rows (right, up, forward) in Bethesda axes. A left
    /// handed basis: z is the distance along the
    /// view direction, positive in front of the camera.
    pub fn view_rows(&self) -> [Vec3; 3] {
        let forward = axes_from_engine(self.forward).normalize();
        let right = forward.cross(axes_from_engine(self.up)).normalize();
        [right, right.cross(forward), forward]
    }
}

/// Size of the half-resolution SSR targets (`W >> 1`, `H >> 1`) of a scene.
pub fn half_extent(width: u32, height: u32) -> UVec2 {
    UVec2::new((width >> 1).max(1), (height >> 1).max(1))
}

/// The water frame constants of the ray-setup pass: the regular ones with the
/// half-resolution texel size in `cb0[0].xy`, the true view rotation in
/// `cb12[0..2]` (so the vertex shader's `view_pos` is a view-space position)
/// and the forward-Z projection in `cb12[4..7]`.
pub fn ray_setup_frame_constants(
    frame: &FrameUniform,
    camera: &SsrCamera,
    half: UVec2,
) -> WaterFrameConstants {
    let mut constants = frame_constants(frame);
    constants.cb0[0].x = 1.0 / half.x as f32;
    constants.cb0[0].y = 1.0 / half.y as f32;

    let rows = camera.view_rows();
    for (offset, row) in rows.iter().enumerate() {
        constants.cb12[CB12_VIEW_ROWS + offset] = row.extend(0.0);
    }
    let clip_from_view = clip_from_view(frame, rows);
    for row in 0..4 {
        constants.cb12[CB12_PROJECTION_ROWS + row] = clip_from_view.row(row);
    }
    constants.cb12[CB12_PROJECTION_ROWS + 2] =
        constants.cb12[CB12_PROJECTION_ROWS + 3] - constants.cb12[CB12_PROJECTION_ROWS + 2];
    constants
}

/// View space (Bethesda units, rotated by `rows`) to the engine's clip space.
fn clip_from_view(frame: &FrameUniform, rows: [Vec3; 3]) -> Mat4 {
    let clip_from_relative =
        frame.view_proj * camera_relative_bethesda(frame.camera_pos.truncate()).inverse();
    let relative_from_view = Mat4::from_mat3(Mat3::from_cols(rows[0], rows[1], rows[2]));
    clip_from_relative * relative_from_view
}

/// The raytracer's and the blur's `cb0[0]`: `(width, height, near, far)`.
pub fn raytrace_constants(half: UVec2, camera: &SsrCamera) -> Vec4 {
    Vec4::new(half.x as f32, half.y as f32, camera.near(), camera.far())
}

/// The Hi-Z `cb0[0]` = `(near, 1, MIN_RAW_DEPTH, _)`: depth-decoding constants that give the
/// reverse-Z infinite projection the linear depth `near / (raw + MIN_RAW_DEPTH)`.
pub fn linear_depth_constants(camera: &SsrCamera) -> Vec4 {
    Vec4::new(camera.near(), 1.0, MIN_RAW_DEPTH, 0.0)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are exact reciprocals and literals"
)]
mod tests {
    use bytemuck::Zeroable;

    use super::*;

    pub(crate) fn camera() -> Camera {
        Camera {
            eye: Vec3::new(30.0, 12.0, -8.0),
            target: Vec3::new(36.0, 11.0, -20.0),
            up: Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 70.0,
            znear: 0.5,
            zfar: 100_000.0,
        }
    }

    pub(crate) fn frame(camera: &Camera) -> FrameUniform {
        let view_proj = camera.build_view_projection_matrix();
        let mut frame = FrameUniform::zeroed();
        frame.view_proj = view_proj;
        frame.inv_view_proj = view_proj.inverse();
        frame.camera_pos = camera.eye.extend(1.0);
        frame.camera_forward = camera.forward().extend(0.0);
        frame.viewport = Vec4::new(1920.0, 1080.0, 1.0 / 1920.0, 1.0 / 1080.0);
        frame.to_sun = Vec4::new(0.0, 1.0, 0.0, 0.99);
        frame
    }

    /// A Bethesda position relative to the camera, `distance` units along the
    /// view direction and `sideways` to its right.
    fn relative_point(camera: &SsrCamera, distance: f32, sideways: f32, rise: f32) -> Vec3 {
        let [right, up, forward] = camera.view_rows();
        forward * distance + right * sideways + up * rise
    }

    fn project(constants: &WaterFrameConstants, view_position: Vec3) -> Vec3 {
        let p = view_position.extend(1.0);
        let row = |index: usize| constants.cb12[CB12_PROJECTION_ROWS + index].dot(p);
        Vec3::new(row(0), row(1), row(2)) / row(3)
    }

    #[test]
    fn the_view_rows_are_orthonormal_with_z_forward() {
        let ssr = SsrCamera::of(&camera());
        let [right, up, forward] = ssr.view_rows();
        for (a, b) in [(right, up), (right, forward), (up, forward)] {
            assert!(a.dot(b).abs() < 1e-6);
        }
        for row in [right, up, forward] {
            assert!((row.length() - 1.0).abs() < 1e-6);
        }
        let engine_forward = axes_from_engine(camera().forward());
        assert!(forward.abs_diff_eq(engine_forward, 1e-6));
    }

    #[test]
    fn the_view_matrix_is_a_pure_rotation_so_view_z_is_the_distance_along_the_view_direction() {
        let ssr = SsrCamera::of(&camera());
        let rows = ssr.view_rows();
        let point = relative_point(&ssr, 420.0, -75.0, 30.0);
        assert!((rows[2].dot(point) - 420.0).abs() < 1e-3);
        assert!((rows[0].dot(point) + 75.0).abs() < 1e-3);
        assert!((rows[1].dot(point) - 30.0).abs() < 1e-3);
    }

    #[test]
    fn the_ray_setup_projection_is_forward_z_zero_at_the_near_plane() {
        let camera = camera();
        let ssr = SsrCamera::of(&camera);
        let constants = ray_setup_frame_constants(&frame(&camera), &ssr, UVec2::new(960, 540));
        let near = ssr.near();
        let at_near = project(&constants, Vec3::new(0.0, 0.0, near));
        assert!(at_near.z.abs() < 1e-5, "{at_near:?}");
        assert!(
            at_near.truncate().length() < 1e-3,
            "the view axis projects to the centre: {at_near:?}"
        );
        let twice = project(&constants, Vec3::new(0.0, 0.0, 2.0 * near));
        assert!((twice.z - 0.5).abs() < 1e-5, "{twice:?}");
        let far = project(&constants, Vec3::new(0.0, 0.0, 1.0e6));
        assert!((far.z - 1.0).abs() < 1e-5, "{far:?}");
    }

    #[test]
    fn the_view_space_projection_agrees_with_the_engine_view_projection() {
        let camera = camera();
        let ssr = SsrCamera::of(&camera);
        let frame = frame(&camera);
        let constants = ray_setup_frame_constants(&frame, &ssr, UVec2::new(960, 540));
        let rows = ssr.view_rows();
        let relative = relative_point(&ssr, 900.0, 140.0, -60.0);
        let view = Vec3::new(
            rows[0].dot(relative),
            rows[1].dot(relative),
            rows[2].dot(relative),
        );
        let engine_point =
            camera.eye + Vec3::new(relative.x, relative.z, -relative.y) / UNITS_FROM_ENGINE;
        let expected = frame.view_proj * engine_point.extend(1.0);
        let ndc = expected.truncate() / expected.w;
        let projected = project(&constants, view);
        assert!(projected.truncate().abs_diff_eq(ndc.truncate(), 1e-4));
        assert!(
            (projected.z - (1.0 - ndc.z)).abs() < 1e-4,
            "forward-Z is one minus reverse-Z"
        );
    }

    #[test]
    fn the_ray_setup_texel_size_is_the_half_resolution_one() {
        let camera = camera();
        let ssr = SsrCamera::of(&camera);
        let constants = ray_setup_frame_constants(&frame(&camera), &ssr, half_extent(1921, 1081));
        assert_eq!(constants.cb0[0].x, 1.0 / 960.0);
        assert_eq!(constants.cb0[0].y, 1.0 / 540.0);
        assert_eq!(
            constants.cb0[0].z, 1.0,
            "the dynamic resolution scale stays"
        );
        assert_eq!(constants.cb0[7].y, 0.2, "the angle threshold travels along");
    }

    #[test]
    fn half_extents_floor_like_a_right_shift_and_never_reach_zero() {
        assert_eq!(half_extent(1920, 1080), UVec2::new(960, 540));
        assert_eq!(half_extent(1921, 1081), UVec2::new(960, 540));
        assert_eq!(half_extent(1, 0), UVec2::new(1, 1));
    }

    #[test]
    fn the_hiz_constants_invert_the_reverse_z_depth_to_a_bethesda_distance() {
        let camera = camera();
        let ssr = SsrCamera::of(&camera);
        let a_b_c = linear_depth_constants(&ssr);
        for distance in [1.0_f32, 20.0, 333.0, 9000.0] {
            let point = camera.eye + camera.forward() * distance;
            let clip = camera.build_view_projection_matrix() * point.extend(1.0);
            let raw = clip.z / clip.w;
            let linear = a_b_c.x / (a_b_c.y * raw + a_b_c.z);
            let expected = distance * UNITS_FROM_ENGINE;
            assert!(
                (linear - expected).abs() <= expected * 1e-4,
                "{distance}: {linear} vs {expected}"
            );
        }
    }

    #[test]
    fn the_raytracer_constants_carry_the_target_size_and_the_clip_planes_in_bethesda_units() {
        let ssr = SsrCamera::of(&camera());
        let cb0 = raytrace_constants(UVec2::new(960, 540), &ssr);
        assert_eq!(cb0, Vec4::new(960.0, 540.0, 5.0, 353_840.0));
    }

    #[test]
    fn the_far_plane_is_the_world_cameras_not_the_engines_zfar() {
        assert_eq!(world_camera_far(Surroundings::Exterior), 353_840.0);
        let camera = camera();
        assert_ne!(
            SsrCamera::of(&camera).far(),
            camera.zfar * UNITS_FROM_ENGINE
        );
        assert_eq!(SsrCamera::of(&camera).with_far(1234.0).far(), 1234.0);
    }

    #[test]
    fn an_interior_far_plane_is_its_fog_clip_distance_capped_at_the_exterior_far() {
        let far = |clip_distance| world_camera_far(Surroundings::Interior { clip_distance });
        assert_eq!(far(None), 353_840.0);
        assert_eq!(far(Some(0.0)), 353_840.0, "a zero clip distance is none");
        assert_eq!(far(Some(4000.0)), 4000.0);
        assert_eq!(far(Some(12_000.0)), 12_000.0, "InstituteTunnel01");
        assert_eq!(
            far(Some(500_000.0)),
            353_840.0,
            "it never lengthens the plane"
        );
    }
}
