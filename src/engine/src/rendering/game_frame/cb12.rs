//! The per-frame `cb12` register block the translated game shaders share (water and effects):
//! view rows, depth un-projection, camera height and fog.
//!
//! Shader space is camera-relative, Bethesda units, Z up; this module converts from engine
//! space (`bethesda_space`). The engine renders reverse-Z with an infinite far plane.

use glam::{Mat4, Vec3, Vec4};

use crate::bethesda_space::{UNITS_FROM_ENGINE, position_from_engine};
use crate::rendering::lighting::fog::FogUniformFields;
use crate::rendering::lighting::frame_uniform::FrameUniform;

/// Register count of `cb12`.
pub const CB12_LEN: usize = 47;

/// Fog range pair used when both distances are 0 and for an unset height band: no fog and no
/// height mix at any reachable distance or height.
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

/// The three fog range pairs in Bethesda units: the distances, and the bottom and top of the near
/// and far height bands (`HeightBand::span`).
struct FogRanges {
    distance: (f32, f32),
    near_height: (f32, f32),
    far_height: (f32, f32),
}

impl FogRanges {
    fn of(fog: &FogUniformFields) -> Self {
        Self {
            distance: if fog.renders() {
                (
                    fog.near_color.w * UNITS_FROM_ENGINE,
                    fog.far_color.w * UNITS_FROM_ENGINE,
                )
            } else {
                UNSET_FOG_RANGE
            },
            near_height: Self::height_band(fog.near_band),
            far_height: Self::height_band(fog.far_band),
        }
    }

    /// A height band of the frame uniform (`share = z * saturate(height * x + y)`, engine units)
    /// as the heights where its share leaves 0 and reaches 1. The registers have no room for
    /// the band's weight, so a band of any weight counts as set.
    fn height_band(band: Vec4) -> (f32, f32) {
        let (scale, offset, weight) = (band.x, band.y, band.z);
        if scale > 0.0 && weight > 0.0 {
            (
                -offset / scale * UNITS_FROM_ENGINE,
                (1.0 - offset) / scale * UNITS_FROM_ENGINE,
            )
        } else {
            UNSET_FOG_RANGE
        }
    }

    /// `cb12[41]` and `cb12[46]`.
    fn registers(&self) -> (Vec4, Vec4) {
        let scale = |(near, far): (f32, f32)| (1.0 / (far - near), near / (far - near));
        let (distance_scale, distance_offset) = scale(self.distance);
        let (near_height_scale, near_height_offset) = scale(self.near_height);
        let (far_height_scale, far_height_offset) = scale(self.far_height);
        (
            Vec4::new(
                distance_scale,
                near_height_scale,
                distance_offset,
                near_height_offset,
            ),
            Vec4::new(
                near_height_scale,
                far_height_scale,
                near_height_offset,
                far_height_offset,
            ),
        )
    }
}

/// The high density scale `cb12[44].w` carries while the fog is off: 1, the same density at every
/// height. Inferred: the register's meaning is itself inferred, and what the game writes there
/// without fog is unknown.
const DISABLED_FOG_HIGH_DENSITY_SCALE: f32 = 1.0;

/// Fog registers `cb12[41..=46]` from the engine's fog: the near colour and its high colour, the
/// far colour and its high colour, and the range pairs. Inferred from the layout, as no public
/// source documents it: `cb12[44].w` takes the high density scale. The colours carry their
/// time-of-day factors; the sunlight term, the fade-in after Near and the ramp to full fog past
/// Max have no register.
fn write_fog(cb12: &mut [Vec4; CB12_LEN], frame: &FrameUniform) {
    let fog = FogUniformFields::of_frame(frame);
    let renders = fog.renders();
    // Disabled fog has power 0 and `pow(0, 0)` would be NaN; any positive
    // power keeps the zero factor of an unset range.
    let power = if renders && fog.params.x > 0.0 {
        fog.params.x
    } else {
        1.0
    };
    let high_density_scale = if renders {
        fog.params.z
    } else {
        DISABLED_FOG_HIGH_DENSITY_SCALE
    };

    let (distance_registers, height_registers) = FogRanges::of(&fog).registers();
    cb12[41] = distance_registers;
    cb12[42] = fog.near_color.truncate().extend(power);
    cb12[43] = fog.near_high_color.truncate().extend(fog.params.y);
    cb12[44] = fog.far_color.truncate().extend(high_density_scale);
    cb12[45] = fog.far_high_color.truncate().extend(0.0);
    cb12[46] = height_registers;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::lighting::fog::{
        Fog, FogColors, FogParams, HeightBand, HeightBands, SunlightTerm,
    };

    fn fog_with(height: HeightBands) -> Fog {
        Fog::Enabled(FogParams {
            colors: FogColors::uniform(Vec3::ONE),
            near: 150.0,
            far: 25_000.0,
            power: 0.25,
            max: 0.85,
            height,
            sunlight: SunlightTerm::NONE,
        })
    }

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

    /// The `saturate(value * scale - offset)` a range register pair gives.
    fn factor(scale: f32, offset: f32, value: f32) -> f32 {
        (value * scale - offset).clamp(0.0, 1.0)
    }

    #[test]
    fn height_bands_reach_the_registers_from_mid_minus_to_mid_plus_range_in_bethesda_units() {
        let height = HeightBands {
            near: HeightBand::new(50.0, 200.0),
            far: HeightBand::new(180.0, 1500.0),
            high_density_scale: 1.0,
        };
        let fields = fog_with(height).uniform_fields();
        let mut frame = frame_fixture();
        fields.write_to(&mut frame);
        let cb12 = frame_cb12(&frame);

        let near_band = |height: f32| factor(cb12[41].y, cb12[41].w, height);
        let far_band = |height: f32| factor(cb12[46].y, cb12[46].w, height);
        for (height, expected) in [(-1500.0, 0.0), (500.0, 0.5), (2500.0, 1.0)] {
            assert!(
                (near_band(height) - expected).abs() < 1e-4,
                "near at {height}"
            );
        }
        for (height, expected) in [(-13_200.0, 0.0), (1800.0, 0.5), (16_800.0, 1.0)] {
            assert!(
                (far_band(height) - expected).abs() < 1e-4,
                "far at {height}"
            );
        }
        for engine_height in [-100.0, 40.0, 900.0] {
            let bethesda_height = engine_height * UNITS_FROM_ENGINE;
            assert!(
                (near_band(bethesda_height) - height.near.share(engine_height)).abs() < 1e-4,
                "the registers and the engine fog agree at {engine_height}"
            );
        }
        assert_eq!((cb12[46].x, cb12[46].z), (cb12[41].y, cb12[41].w));
    }

    #[test]
    fn disabled_fog_carries_the_neutral_high_density_scale() {
        let mut frame = frame_fixture();
        Fog::Disabled.uniform_fields().write_to(&mut frame);
        assert!((frame_cb12(&frame)[44].w - DISABLED_FOG_HIGH_DENSITY_SCALE).abs() < f32::EPSILON);
    }

    #[test]
    fn enabled_fog_carries_its_own_high_density_scale() {
        let mut frame = frame_fixture();
        fog_with(HeightBands {
            far: HeightBand::new(180.0, 1500.0),
            high_density_scale: 0.25,
            ..HeightBands::NONE
        })
        .uniform_fields()
        .write_to(&mut frame);
        assert!((frame_cb12(&frame)[44].w - 0.25).abs() < 1e-6);
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
