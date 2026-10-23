//! The per-frame `cb12` register block the translated game shaders share (water and effects):
//! view rows, depth un-projection, camera height and fog.
//!
//! Shader space is camera-relative, Bethesda units, Z up; this module converts from engine
//! space (`bethesda_space`). The engine renders reverse-Z with an infinite far plane.

use glam::{Mat4, Vec3, Vec4};

use crate::bethesda_space::{UNITS_FROM_ENGINE, position_from_engine};
use crate::rendering::lighting::frame_uniform::FrameUniform;

/// Register count of `cb12`.
pub const CB12_LEN: usize = 47;

/// `cb12[44].w`: strength of the far fog's height-fog contribution. With the engine's weather
/// having no height-fog ranges the mix is 0 and the value is inert.
pub const FAR_FOG_HEIGHT_STRENGTH: f32 = 1.0;

/// Fog range pair used when both distances are 0: no fog at any reachable distance.
pub const UNSET_FOG_RANGE: (f32, f32) = (1.0e8, 1.0e9);

/// Binding registers of the two depth un-projection matrices: [`DEPTH_NEAR_RANGE_REGISTER`]
/// for depths up to [`DEPTH_RANGE_SPLIT`] (rescaled by [`NEAR_RANGE_SCALE`]) and
/// [`DEPTH_FAR_RANGE_REGISTER`] for the rest (rescaled by [`FAR_RANGE_SCALE`]). The engine has
/// one reverse-Z range with an infinite far plane and passes the raw depth, so both matrices
/// are the engine's inverse view-projection with that rescaling folded in; depth is clamped to
/// `MIN_SCENE_DEPTH` (the horizon un-projects to infinity) by shifting the matrices.
///
/// For a camera-relative frame the z column of rows 0..=2 of the far-range matrix is zero, so
/// those rows also map an undivided clip position straight to the camera-relative Bethesda
/// position (how the effect vertex stage reads them).
pub const DEPTH_FAR_RANGE_REGISTER: usize = 20;
pub const DEPTH_NEAR_RANGE_REGISTER: usize = 24;
pub const DEPTH_RANGE_SPLIT: f32 = 0.01;
pub const NEAR_RANGE_SCALE: f32 = 100.0;
pub const FAR_RANGE_SCALE: f32 = 1.01;
pub const MIN_SCENE_DEPTH: f32 = 1e-7;

/// `cb12[14]`: dot row giving the camera-relative height (Bethesda z).
const HEIGHT_ROW_REGISTER: usize = 14;
/// `cb12[35].xyz`: the camera position in Bethesda units.
const CAMERA_POSITION_REGISTER: usize = 35;

/// Bethesda-axes, Bethesda-unit, camera-relative transform of engine space.
pub fn camera_relative_bethesda(camera: Vec3) -> Mat4 {
    let units = UNITS_FROM_ENGINE;
    let axes = Mat4::from_cols(
        Vec4::new(units, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, units, 0.0),
        Vec4::new(0.0, -units, 0.0, 0.0),
        Vec4::W,
    );
    axes * Mat4::from_translation(-camera)
}

fn depth_unprojection_rows(inverse_view_projection: Mat4) -> ([Vec4; 4], [Vec4; 4]) {
    let mut far = [Vec4::ZERO; 4];
    let mut near = [Vec4::ZERO; 4];
    for row in 0..4 {
        let engine = inverse_view_projection.row(row);
        let shifted_w = engine.w + engine.z * MIN_SCENE_DEPTH;
        let far_z = engine.z / FAR_RANGE_SCALE;
        far[row] = Vec4::new(
            engine.x,
            engine.y,
            far_z,
            shifted_w + DEPTH_RANGE_SPLIT * far_z,
        );
        near[row] = Vec4::new(engine.x, engine.y, engine.z / NEAR_RANGE_SCALE, shifted_w);
    }
    (far, near)
}

/// `cb12` for this frame's engine state.
pub fn frame_cb12(frame: &FrameUniform) -> [Vec4; CB12_LEN] {
    let camera = frame.camera_pos.truncate();
    let inverse_view_projection = camera_relative_bethesda(camera) * frame.inv_view_proj;

    let mut cb12 = [Vec4::ZERO; CB12_LEN];
    // View space is the camera-relative world itself.
    cb12[0] = Vec4::X;
    cb12[1] = Vec4::Y;
    cb12[2] = Vec4::Z;
    cb12[HEIGHT_ROW_REGISTER] = Vec4::Z;
    let (far_rows, near_rows) = depth_unprojection_rows(inverse_view_projection);
    cb12[DEPTH_FAR_RANGE_REGISTER..DEPTH_FAR_RANGE_REGISTER + 4].copy_from_slice(&far_rows);
    cb12[DEPTH_NEAR_RANGE_REGISTER..DEPTH_NEAR_RANGE_REGISTER + 4].copy_from_slice(&near_rows);
    cb12[CAMERA_POSITION_REGISTER] = position_from_engine(camera).extend(0.0);
    write_fog(&mut cb12, frame);
    cb12
}

/// The three fog distance pairs: the weather's day near/far,
/// and the two height-fog pairs `(a - b, a + b)` and `(c - d, c + d)`. The
/// engine's weather has no height-fog data, so both are the unset pair.
struct FogRanges {
    main: (f32, f32),
    mid: (f32, f32),
    high: (f32, f32),
}

impl FogRanges {
    fn of_main(near: f32, far: f32) -> Self {
        Self {
            main: Self::substitute_unset(near, far),
            mid: UNSET_FOG_RANGE,
            high: UNSET_FOG_RANGE,
        }
    }

    fn substitute_unset(near: f32, far: f32) -> (f32, f32) {
        if near == 0.0 && far == 0.0 {
            UNSET_FOG_RANGE
        } else {
            (near, far)
        }
    }

    /// `cb12[41]` and `cb12[46]`.
    fn registers(&self) -> (Vec4, Vec4) {
        let scale = |(near, far): (f32, f32)| (1.0 / (far - near), near / (far - near));
        let (main_scale, main_offset) = scale(self.main);
        let (mid_scale, mid_offset) = scale(self.mid);
        let (high_scale, high_offset) = scale(self.high);
        (
            Vec4::new(main_scale, mid_scale, main_offset, mid_offset),
            Vec4::new(mid_scale, high_scale, mid_offset, high_offset),
        )
    }
}

/// Fog registers `cb12[41..=46]` from the engine's
/// parametric fog. The engine has one near and one far colour, no height-fog
/// colours or ranges: the "high" colours repeat them and the height pairs are
/// unset, so the height mix is 0 and they cannot show.
fn write_fog(cb12: &mut [Vec4; CB12_LEN], frame: &FrameUniform) {
    let near_color = frame.fog_near_color.truncate();
    let far_color = frame.fog_far_color.truncate();
    let near = frame.fog_near_color.w * UNITS_FROM_ENGINE;
    let far = frame.fog_far_color.w * UNITS_FROM_ENGINE;
    // Disabled fog has power 0 and `pow(0, 0)` would be NaN; any positive
    // power keeps the zero factor of an unset range.
    let power = if frame.fog_params.x > 0.0 {
        frame.fog_params.x
    } else {
        1.0
    };
    let max_opacity = frame.fog_params.y;

    let (distance_registers, height_registers) = FogRanges::of_main(near, far).registers();
    cb12[41] = distance_registers;
    cb12[42] = near_color.extend(power);
    cb12[43] = near_color.extend(max_opacity);
    cb12[44] = far_color.extend(FAR_FOG_HEIGHT_STRENGTH);
    cb12[45] = far_color.extend(0.0);
    cb12[46] = height_registers;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_fixture() -> FrameUniform {
        let mut frame: FrameUniform = bytemuck::Zeroable::zeroed();
        let eye = Vec3::new(-35.0, 12.0, 80.0);
        let view = Mat4::look_to_rh(eye, Vec3::new(-0.4, 0.1, -1.0).normalize(), Vec3::Y);
        let projection = Mat4::perspective_infinite_reverse_rh(1.1, 16.0 / 9.0, 0.5);
        frame.view_proj = projection * view;
        frame.inv_view_proj = frame.view_proj.inverse();
        frame.camera_pos = eye.extend(1.0);
        frame
    }

    /// The effect vertex stage's use of rows 20..=22: dot products with the undivided clip
    /// position give the camera-relative Bethesda position.
    #[test]
    fn far_range_rows_map_an_undivided_clip_position_to_camera_relative_bethesda_units() {
        let frame = frame_fixture();
        let cb12 = frame_cb12(&frame);
        let camera = frame.camera_pos.truncate();
        for point in [
            Vec3::new(-40.0, 10.0, 70.0),
            Vec3::new(-60.0, 15.0, 20.0),
            Vec3::new(-35.5, 12.0, 79.0),
        ] {
            let clip = frame.view_proj * point.extend(1.0);
            let relative = Vec3::new(
                cb12[DEPTH_FAR_RANGE_REGISTER].dot(clip),
                cb12[DEPTH_FAR_RANGE_REGISTER + 1].dot(clip),
                cb12[DEPTH_FAR_RANGE_REGISTER + 2].dot(clip),
            );
            let expected = position_from_engine(point - camera);
            assert!(
                relative.abs_diff_eq(expected, 1e-3 * expected.length().max(1.0)),
                "{point:?}: expected {expected:?}, got {relative:?}"
            );
        }
    }

    #[test]
    fn camera_relative_bethesda_maps_the_camera_to_the_origin_and_scales_to_bethesda_units() {
        let camera = Vec3::new(3.0, 4.0, 5.0);
        let transform = camera_relative_bethesda(camera);
        assert!(
            transform
                .transform_point3(camera)
                .abs_diff_eq(Vec3::ZERO, 1e-6)
        );
        let offset = Vec3::new(1.0, 2.0, -3.0);
        assert!(
            transform
                .transform_point3(camera + offset)
                .abs_diff_eq(Vec3::new(10.0, 30.0, 20.0), 1e-5)
        );
    }
}
