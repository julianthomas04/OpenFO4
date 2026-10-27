//! The one place that fills the binding registers of the translated water shaders
//! (wrapped by `shaders/water_wrapper.wgsl`): WATR fields, frame state and per-draw geometry
//! in, `cb0` / `cb1` / `cb2` / `cb12` out.
//!
//! Every mapping is the CPU-side fill of one register array. Values with no engine source use
//! neutral defaults.
//!
//! Shader space is camera-relative, Bethesda units, Z up; this module converts
//! from engine space (`bethesda_space`).

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::assets::db::waters::{Rgba8, WaterData, WaterNoiseLayer};
use crate::bethesda_space::{axes_from_engine, position_from_engine};
pub(super) use crate::rendering::game_frame::cb12::camera_relative_bethesda;
use crate::rendering::game_frame::cb12::{CB12_LEN, frame_cb12};
#[cfg(test)]
use crate::rendering::game_frame::cb12::{
    DEPTH_FAR_RANGE_REGISTER, DEPTH_NEAR_RANGE_REGISTER, DEPTH_RANGE_SPLIT, FAR_RANGE_SCALE,
    NEAR_RANGE_SCALE, UNSET_FOG_RANGE,
};
use crate::rendering::lighting::frame_uniform::{FrameUniform, TIME_WRAP_SECONDS};

/// Register counts of the binding slots the translated water shaders read.
const CB0_LEN: usize = 8;
const PS_CB1_LEN: usize = 14;
const PS_CB2_LEN: usize = 5;
const VS_CB2_LEN: usize = 8;
const VS_CB1_LEN: usize = 3;

/// `powf(byte / 255, 2.2)` colour conversion of the WATR fields.
const COLOUR_GAMMA: f32 = 2.2;
const BYTE_TO_UNIT: f32 = 0.003_921_568_6;

/// ANAM (opacity) is stored scaled by 0.01.
const OPACITY_SCALE: f32 = 0.01;
/// `cb1[8].w`: filler with no engine role.
const CB1_8_FILLER_W: f32 = 250.0;
/// `cb1[10].y`: filler with no engine role.
const CB1_10_FILLER_Y: f32 = 0.5;
/// Clamp of the layer-1 noise falloff.
const NOISE_FALLOFF_RANGE: (f32, f32) = (1.0, 8191.0);
/// Lower bound of the depth amount (same row).
const DEPTH_AMOUNT_MIN: f32 = 1.0;
/// Degrees to radians for the scroll direction.
const DEGREES_TO_RADIANS: f32 = 0.017_453_292;

/// `cb0[0].zw` / `cb0[1].xy`: the dynamic-resolution scale globals, both 1.0.
const DYNAMIC_RESOLUTION_SCALE: f32 = 1.0;
/// `cb0[3].w`: exponent of the fog's sun in-scatter.
pub const SUN_SCATTER_EXPONENT: f32 = 8.0;
/// `cb0[7].x`: INI `[Sslr] fWaterBlurRatio`, default 0.9.
pub const WATER_BLUR_RATIO: f32 = 0.9;
/// `cb0[7].y`: INI `[SSLR] fSSLRAngleThreshold`, default 0.2.
pub const SSLR_ANGLE_THRESHOLD: f32 = 0.2;

/// `cb0` and `cb12`: constants shared by every water draw of a frame.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterFrameConstants {
    pub cb0: [Vec4; CB0_LEN],
    pub cb12: [Vec4; CB12_LEN],
}

const _: () = assert!(std::mem::size_of::<WaterFrameConstants>() == (CB0_LEN + CB12_LEN) * 16);

/// Per-draw registers: vertex and pixel stage `cb1` / `cb2`, which are
/// separate buffers in D3D.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterDrawConstants {
    pub vs_cb2: [Vec4; VS_CB2_LEN],
    pub ps_cb2: [Vec4; PS_CB2_LEN],
    pub vs_cb1: [Vec4; VS_CB1_LEN],
    pub ps_cb1: [Vec4; PS_CB1_LEN],
    pub padding: [Vec4; 2],
}

/// Dynamic-offset stride of one draw's constants (the WebGPU minimum uniform
/// offset alignment is 256).
pub const DRAW_CONSTANTS_STRIDE: u64 = 512;
const _: () = assert!(std::mem::size_of::<WaterDrawConstants>() as u64 == DRAW_CONSTANTS_STRIDE);

pub(super) fn gamma_rgb(color: Rgba8) -> Vec3 {
    let channel = |byte: u8| (f32::from(byte) * BYTE_TO_UNIT).powf(COLOUR_GAMMA);
    Vec3::new(channel(color[0]), channel(color[1]), channel(color[2]))
}

/// What one normal layer contributes to the scroll and the tile size.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct NoiseLayer {
    /// WATR wind direction in degrees.
    pub wind_direction_degrees: f32,
    pub wind_speed: f32,
    /// World units per texture repeat (`UV Scale`, raw).
    pub uv_scale: f32,
}

impl NoiseLayer {
    fn of(layer: &WaterNoiseLayer) -> Self {
        Self {
            wind_direction_degrees: layer.wind_direction,
            wind_speed: layer.wind_speed,
            uv_scale: layer.uv_scale,
        }
    }
}

/// Wraps a scroll offset into `[0, 1]`: values above 1 or below 0 wrap by their fractional
/// part, a value of exactly 1 is kept.
fn wrap_scroll(value: f32) -> f32 {
    if value > 1.0 {
        value % 1.0
    } else if value < 0.0 {
        value % 1.0 + 1.0
    } else {
        value
    }
}

/// The three layers' scroll offsets, accumulated per frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NormalScroll {
    offsets: [Vec2; 3],
    last_time: Option<f32>,
}

impl NormalScroll {
    /// One update of `dt` seconds: `offset.x += sin(th) * (dt * speed)`,
    /// `offset.y += cos(th) * (dt * speed)`, wrapped into `[0, 1]`.
    pub fn step(&mut self, layers: &[NoiseLayer; 3], dt: f32) {
        for (offset, layer) in self.offsets.iter_mut().zip(layers) {
            let (sin, cos) = (layer.wind_direction_degrees * DEGREES_TO_RADIANS).sin_cos();
            let distance = dt * layer.wind_speed;
            offset.x = wrap_scroll(offset.x + sin * distance);
            offset.y = wrap_scroll(offset.y + cos * distance);
        }
    }

    /// Advances to the frame clock `time` (seconds, wrapped at
    /// `TIME_WRAP_SECONDS`). Calling it twice with the same time is a no-op, so
    /// every surface of a water may ask each frame; the first call only starts
    /// the clock.
    pub fn advance_to(&mut self, layers: &[NoiseLayer; 3], time: f32) {
        let dt = self
            .last_time
            .map_or(0.0, |last| (time - last).rem_euclid(TIME_WRAP_SECONDS));
        self.last_time = Some(time);
        self.step(layers, dt);
    }

    pub fn offsets(&self) -> [Vec2; 3] {
        self.offsets
    }
}

/// `cb1[2]`: the deep colour and reflectivity of the water of the current
/// worldspace, not of the surface being drawn.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FarField {
    pub deep_color: Vec3,
    pub reflectivity: f32,
}

impl FarField {
    /// Interior (no worldspace): `(0, 0, 0, 1)`.
    pub const INTERIOR: Self = Self {
        deep_color: Vec3::ZERO,
        reflectivity: 1.0,
    };

    pub fn of_water(data: &WaterData) -> Self {
        Self {
            deep_color: gamma_rgb(data.fog_deep_color),
            reflectivity: data.physical_reflectivity_amount,
        }
    }
}

/// Everything a WATR contributes, resolved once per record, plus the record's scroll state.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterMaterial {
    /// Pixel `cb1` as the material image holds it: rows 2 to 5 are frame
    /// state and stay zero here, colours are not yet tinted.
    cb1: [Vec4; PS_CB1_LEN],
    /// Underwater colour, swaps into `cb1[0]` under water.
    underwater_color: Vec3,
    /// WATR reflection colour: the interior sky gradient.
    reflection_color: Vec3,
    /// This WATR as the worldspace's water.
    far_field: FarField,
    layers: [NoiseLayer; 3],
    scroll: NormalScroll,
}

impl WaterMaterial {
    /// The WATR fields mapped onto the material image and from there onto the
    /// pixel `cb1` registers.
    pub fn from_watr(data: &WaterData) -> Self {
        let layers = data.noise_layers();
        let mut cb1 = [Vec4::ZERO; PS_CB1_LEN];
        cb1[0] = gamma_rgb(data.fog_shallow_color).extend(data.specular_sun_sparkle_power);
        cb1[1] = gamma_rgb(data.fog_deep_color).extend(data.specular_sun_specular_magnitude);
        cb1[6] = gamma_rgb(data.silt_light_color).extend(data.silt_amount);
        cb1[7] = gamma_rgb(data.silt_dark_color).extend(0.0);
        cb1[8] = Vec4::new(
            data.specular_sun_specular_power,
            data.physical_reflectivity_amount,
            f32::from(data.opacity) * OPACITY_SCALE,
            CB1_8_FILLER_W,
        );
        cb1[9] = Vec4::new(
            layers[0].amplitude_scale,
            layers[1].amplitude_scale,
            layers[2].amplitude_scale,
            data.physical_displacement_simulator_dampener,
        );
        cb1[10] = Vec4::new(
            layers[0]
                .noise_falloff
                .clamp(NOISE_FALLOFF_RANGE.0, NOISE_FALLOFF_RANGE.1),
            CB1_10_FILLER_Y,
            data.specular_sun_sparkle_magnitude,
            data.fog_depth_amount.max(DEPTH_AMOUNT_MIN),
        );
        cb1[11] = Vec4::new(
            data.physical_surface_effect_falloff,
            data.physical_normal_magnitude,
            data.physical_shallow_normal_falloff,
            data.physical_deep_normal_falloff,
        );
        cb1[12] = Vec4::new(
            data.fog_alpha_shallow_range,
            data.fog_alpha_deep_range,
            data.fog_shallow_alpha,
            data.fog_deep_alpha,
        );
        cb1[13] = Vec4::new(
            data.fog_color_shallow_range,
            data.fog_color_deep_range,
            data.physical_fresnel_amount,
            data.specular_interior_specular_power,
        );
        Self {
            cb1,
            underwater_color: gamma_rgb(data.fog_underwater_color),
            reflection_color: gamma_rgb(data.physical_reflection_color),
            far_field: FarField::of_water(data),
            layers: layers.map(NoiseLayer::of),
            scroll: NormalScroll::default(),
        }
    }

    pub fn far_field(&self) -> FarField {
        self.far_field
    }

    /// Advances the layer scroll to the frame clock (see
    /// [`NormalScroll::advance_to`]); call once per frame for every water in use.
    pub fn advance_scroll(&mut self, time: f32) {
        self.scroll.advance_to(&self.layers, time);
    }
}

/// Frame state the pixel `cb1` rows depend on besides the WATR.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WaterEnvironment {
    /// `cb1[2]` source: the current worldspace's water.
    pub far_field: FarField,
    /// The water multiplier colour applied to `cb1[0..=2]`, `[6]`, `[7]` and the interior
    /// `[3..=5]`. Which tint applies depends on a weather flag that the engine's
    /// weather does not resolve yet, so callers pass [`Self::NEUTRAL_TINT`], the `(1, 1, 1)`
    /// value for a clear flag.
    pub tint: Vec3,
}

impl WaterEnvironment {
    pub const NEUTRAL_TINT: Vec3 = Vec3::ONE;

    pub fn new(far_field: FarField) -> Self {
        Self {
            far_field,
            tint: Self::NEUTRAL_TINT,
        }
    }
}

/// Vertex stage `cb1`: scroll offsets of the three layers (`[0].xy`, `[0].zw`,
/// `[1].xy`) and their raw UV scale (`[2].xyz`).
fn vertex_cb1(scroll: &NormalScroll, layers: &[NoiseLayer; 3]) -> [Vec4; VS_CB1_LEN] {
    let [first, second, third] = scroll.offsets();
    let [a, b, c] = layers;
    [
        Vec4::new(first.x, first.y, second.x, second.y),
        Vec4::new(third.x, third.y, 0.0, 0.0),
        Vec4::new(a.uv_scale, b.uv_scale, c.uv_scale, 0.0),
    ]
}

/// `cb0[3].xyz`: the sun light's linear colour times its dimmer. Unlike the
/// lighting shader, the water fill does not multiply in the image space's
/// sunlight scale, so the scale folded
/// into the frame's sun illuminance is divided out again.
fn unscaled_sun_colour(frame: &FrameUniform) -> Vec3 {
    let illuminance = frame.sun_illuminance.truncate();
    let sunlight_scale = frame.sun_illuminance.w;
    if sunlight_scale > 0.0 {
        illuminance / sunlight_scale
    } else {
        illuminance
    }
}

/// `cb0` and `cb12` for this frame's engine state.
pub fn frame_constants(frame: &FrameUniform) -> WaterFrameConstants {
    let texel = Vec2::new(frame.viewport.z, frame.viewport.w);

    let mut cb0 = [Vec4::ZERO; CB0_LEN];
    cb0[0] = Vec4::new(
        texel.x,
        texel.y,
        DYNAMIC_RESOLUTION_SCALE.recip(),
        DYNAMIC_RESOLUTION_SCALE.recip(),
    );
    cb0[1] = Vec4::new(
        DYNAMIC_RESOLUTION_SCALE - texel.x,
        DYNAMIC_RESOLUTION_SCALE - texel.y,
        0.0,
        0.0,
    );
    cb0[2] = axes_from_engine(frame.to_sun.truncate()).extend(frame.daylight.x);
    cb0[3] = unscaled_sun_colour(frame).extend(SUN_SCATTER_EXPONENT);
    cb0[7] = Vec4::new(WATER_BLUR_RATIO, SSLR_ANGLE_THRESHOLD, 0.0, 0.0);

    WaterFrameConstants {
        cb0,
        cb12: frame_cb12(frame),
    }
}

/// What the geometry registers need to know about one draw.
#[derive(Debug, Copy, Clone)]
pub struct GeometryInputs {
    /// Engine-space world matrix of the surface.
    pub world: Mat4,
    pub view_projection: Mat4,
    pub camera: Vec3,
    /// A point on the water plane in engine space: the plane distance is
    /// `dot(N, geometry world translation)`, so this is the translation of `world`.
    pub plane_point: Vec3,
    /// Descriptor bit 0x2: the layers use the mesh uv instead of the world xy.
    pub mesh_uv: bool,
    /// Descriptor bit 0x20: no direct sun.
    pub interior: bool,
}

/// `cb2`: the vertex rows and the pixel stage's
/// plane `(N, dot(N, camera) - d)`. The plane normal is the
/// surface's local +Y, i.e. engine up for a flat mesh. The point-light rows
/// `cb2[6..]` stay zero: the water shading has no point lights yet.
fn geometry_registers(inputs: &GeometryInputs) -> ([Vec4; VS_CB2_LEN], [Vec4; PS_CB2_LEN]) {
    let world_to_shader = camera_relative_bethesda(inputs.camera) * inputs.world;
    let clip = inputs.view_projection * inputs.world;
    let mut vs_cb2 = [Vec4::ZERO; VS_CB2_LEN];
    for (row, register) in vs_cb2.iter_mut().take(3).enumerate() {
        *register = world_to_shader.row(row);
    }
    for row in 0..4 {
        vs_cb2[3 + row] = clip.row(row);
    }
    vs_cb2[7] = Vec4::new(f32::from(u8::from(inputs.mesh_uv)), 0.0, 0.0, 0.0);

    let normal = axes_from_engine(inputs.world.transform_vector3(Vec3::Y).normalize());
    let plane_distance = normal.dot(position_from_engine(inputs.plane_point));
    let camera_distance = normal.dot(position_from_engine(inputs.camera));
    let mut ps_cb2 = [Vec4::ZERO; PS_CB2_LEN];
    ps_cb2[4] = normal.extend(camera_distance - plane_distance);

    (vs_cb2, ps_cb2)
}

/// Multiplies the rgb of a row by the water tint (`w` by 1.0).
fn tinted(row: Vec4, tint: Vec3) -> Vec4 {
    (row.truncate() * tint).extend(row.w)
}

/// Registers of the sun glint (`cb1[1].w`) and sparkle (`cb1[10].z`) strengths; an interior has
/// no direct sun, so both are zero there.
const SPECULAR_STRENGTH_ROW: usize = 1;
const SPARKLE_STRENGTH_ROW: usize = 10;

/// The pixel `cb1` of one draw: the material image with the per-frame rows applied.
fn pixel_cb1(
    material: &WaterMaterial,
    frame: &FrameUniform,
    environment: &WaterEnvironment,
    interior: bool,
) -> [Vec4; PS_CB1_LEN] {
    let underwater = frame.time_water.z != 0.0;
    let tint = environment.tint;
    let mut cb1 = material.cb1;

    if underwater {
        cb1[0] = material.underwater_color.extend(cb1[0].w);
    }
    let far_field = if interior {
        FarField::INTERIOR
    } else {
        environment.far_field
    };
    cb1[2] = far_field.deep_color.extend(far_field.reflectivity);
    for row in [0, 1, 2, 6, 7] {
        cb1[row] = tinted(cb1[row], tint);
    }

    let gradient = if interior {
        [tinted(material.reflection_color.extend(0.0), tint); 3]
    } else {
        // Horizon, sky lower, sky upper; the frame's sky colours already carry the HDR sky scale.
        [frame.sky_horizon, frame.sky_ground, frame.sky_zenith]
            .map(|color| color.truncate().extend(0.0))
    };
    cb1[3..=5].copy_from_slice(&gradient);
    if interior {
        cb1[SPECULAR_STRENGTH_ROW].w = 0.0;
        cb1[SPARKLE_STRENGTH_ROW].z = 0.0;
    }
    cb1
}

pub fn draw_constants(
    material: &WaterMaterial,
    frame: &FrameUniform,
    geometry: &GeometryInputs,
    environment: &WaterEnvironment,
) -> WaterDrawConstants {
    let (vs_cb2, ps_cb2) = geometry_registers(geometry);
    WaterDrawConstants {
        vs_cb2,
        ps_cb2,
        vs_cb1: vertex_cb1(&material.scroll, &material.layers),
        ps_cb1: pixel_cb1(material, frame, environment, geometry.interior),
        padding: [Vec4::ZERO; 2],
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "registers are copies or exact products of fixture values"
)]
#[allow(
    clippy::unreadable_literal,
    reason = "the fixture values are the record's own digits"
)]
mod tests {
    use super::*;
    use crate::rendering::water::test_fixtures::synthetic_water;
    use glam::UVec4;

    fn ocean() -> WaterData {
        synthetic_water()
    }

    fn ocean_material() -> WaterMaterial {
        WaterMaterial::from_watr(&ocean())
    }

    /// `powf(byte / 255, 2.2)` written out independently of `gamma_rgb`.
    fn gamma(bytes: [u8; 3]) -> Vec3 {
        let channel = |b: u8| (f32::from(b) / 255.0).powf(2.2);
        Vec3::new(channel(bytes[0]), channel(bytes[1]), channel(bytes[2]))
    }

    fn assert_close(actual: Vec3, expected: Vec3, context: &str) {
        assert!(
            actual.abs_diff_eq(expected, 1e-6),
            "{context}: {actual:?} vs {expected:?}"
        );
    }

    fn frame_fixture() -> FrameUniform {
        let mut frame: FrameUniform = bytemuck::Zeroable::zeroed();
        let eye = Vec3::new(120.0, 35.0, -48.0);
        let view = Mat4::look_to_rh(eye, Vec3::new(0.3, -0.2, -1.0).normalize(), Vec3::Y);
        let projection = Mat4::perspective_infinite_reverse_rh(1.2, 16.0 / 9.0, 0.1);
        frame.view_proj = projection * view;
        frame.inv_view_proj = frame.view_proj.inverse();
        frame.camera_pos = eye.extend(1.0);
        frame.viewport = Vec4::new(1920.0, 1080.0, 1.0 / 1920.0, 1.0 / 1080.0);
        frame.to_sun = Vec3::new(0.0, 1.0, 0.0).extend(0.99);
        frame.daylight = Vec4::new(1.0, 0.0, 0.0, 0.0);
        frame.sun_illuminance = Vec4::new(3.0, 2.0, 1.0, 0.0);
        frame.sky_zenith = Vec4::new(0.1, 0.2, 0.9, 0.0);
        frame.sky_horizon = Vec4::new(0.6, 0.7, 0.8, 0.0);
        frame.sky_ground = Vec4::new(0.2, 0.25, 0.3, 0.0);
        frame.fog_near_color = Vec4::new(0.5, 0.5, 0.6, 10.0);
        frame.fog_far_color = Vec4::new(0.7, 0.7, 0.8, 110.0);
        frame.fog_params = Vec4::new(2.0, 0.9, 0.95, 0.0);
        frame.fog_near_high_color = Vec4::new(0.3, 0.35, 0.4, 0.0);
        frame.fog_far_high_color = Vec4::new(0.45, 0.5, 0.6, 0.0);
        frame.fog_near_band = Vec4::new(1.0 / 128.0, 0.5, 1.0, 0.0);
        frame.flags = UVec4::new(0, 0, 13, 0);
        frame
    }

    fn geometry_fixture(world: Mat4) -> GeometryInputs {
        let frame = frame_fixture();
        GeometryInputs {
            world,
            view_projection: frame.view_proj,
            camera: frame.camera_pos.truncate(),
            plane_point: world.w_axis.truncate(),
            mesh_uv: false,
            interior: false,
        }
    }

    fn other_water_far_field() -> FarField {
        FarField {
            deep_color: Vec3::new(0.01, 0.02, 0.03),
            reflectivity: 0.77,
        }
    }

    fn exterior_cb1() -> [Vec4; PS_CB1_LEN] {
        let environment = WaterEnvironment::new(other_water_far_field());
        pixel_cb1(&ocean_material(), &frame_fixture(), &environment, false)
    }

    // --- cb1 ------------------------------------------------

    #[test]
    fn shallow_and_deep_rows_are_gamma_colours_with_sparkle_power_and_specular_magnitude() {
        let cb1 = exterior_cb1();
        // Rows cb1[0] / cb1[1]: shallow and deep colour, w from
        // Sun Sparkle Power / Sun Specular Magnitude.
        assert_close(cb1[0].truncate(), gamma([10, 20, 30]), "cb1[0].xyz");
        assert_eq!(cb1[0].w, 10000.0);
        assert_close(cb1[1].truncate(), gamma([40, 50, 60]), "cb1[1].xyz");
        assert_eq!(cb1[1].w, 10.0);
    }

    #[test]
    fn silt_rows_are_gamma_colours_with_the_silt_amount() {
        let cb1 = exterior_cb1();
        assert_close(cb1[6].truncate(), gamma([100, 120, 140]), "cb1[6].xyz");
        assert_eq!(cb1[6].w, 0.5);
        assert_close(cb1[7].truncate(), gamma([20, 30, 40]), "cb1[7].xyz");
        assert_eq!(cb1[7].w, 0.0);
    }

    #[test]
    fn scalar_rows_follow_the_material_image() {
        let cb1 = exterior_cb1();
        // cb1[8]: specular power, reflectivity, ANAM * 0.01, constant 250.
        assert_eq!(cb1[8], Vec4::new(2000.0, 0.3, 60.0 * 0.01, 250.0));
        // cb1[9]: amplitudes and the displacement dampener.
        assert_eq!(cb1[9], Vec4::new(0.5, 0.6, 0.7, 0.95));
        // cb1[10]: layer-1 falloff, constant 0.5, sparkle magnitude, depth.
        assert_eq!(cb1[10], Vec4::new(5000.0, 0.5, 100.0, 2000.0));
        // cb1[11].
        assert_eq!(cb1[11], Vec4::new(0.7, 0.5, 0.9, 0.8));
        // cb1[12].
        assert_eq!(cb1[12], Vec4::new(1.0, 0.25, 0.0, 1.0));
        // cb1[13]: colour ramp, fresnel, interior specular power.
        assert_eq!(cb1[13], Vec4::new(0.5, 0.75, 0.1, 200.0));
    }

    #[test]
    fn falloff_and_depth_amount_are_clamped() {
        let mut data = ocean();
        data.noise_layer_one.noise_falloff = 20_000.0;
        data.fog_depth_amount = 0.25;
        let cb1 = WaterMaterial::from_watr(&data).cb1;
        assert_eq!(cb1[10].x, 8191.0);
        assert_eq!(cb1[10].w, 1.0);
        data.noise_layer_one.noise_falloff = 0.0;
        assert_eq!(WaterMaterial::from_watr(&data).cb1[10].x, 1.0);
    }

    #[test]
    fn cb1_2_is_the_worldspace_waters_deep_colour_and_reflectivity_not_the_surfaces() {
        let cb1 = exterior_cb1();
        assert_close(cb1[2].truncate(), Vec3::new(0.01, 0.02, 0.03), "cb1[2].xyz");
        assert_eq!(cb1[2].w, 0.77);
    }

    #[test]
    fn a_water_is_its_own_far_field_when_it_is_the_worldspace_water() {
        let material = ocean_material();
        assert_close(
            material.far_field().deep_color,
            gamma([40, 50, 60]),
            "far field",
        );
        assert_eq!(material.far_field().reflectivity, 0.3);
    }

    #[test]
    fn interior_far_field_is_black_with_reflectivity_one() {
        let environment = WaterEnvironment::new(other_water_far_field());
        let cb1 = pixel_cb1(&ocean_material(), &frame_fixture(), &environment, true);
        assert_eq!(cb1[2], Vec4::new(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn tint_multiplies_the_colour_rows_but_not_their_w() {
        let tint = Vec3::new(0.5, 2.0, 0.25);
        let environment = WaterEnvironment {
            tint,
            ..WaterEnvironment::new(other_water_far_field())
        };
        let plain = exterior_cb1();
        let tinted_cb1 = pixel_cb1(&ocean_material(), &frame_fixture(), &environment, false);
        for row in [0, 1, 2, 6, 7] {
            assert_close(
                tinted_cb1[row].truncate(),
                plain[row].truncate() * tint,
                &format!("cb1[{row}]"),
            );
            assert_eq!(tinted_cb1[row].w, plain[row].w, "cb1[{row}].w");
        }
        for row in [3, 4, 5, 8, 9, 10, 11, 12, 13] {
            assert_eq!(tinted_cb1[row], plain[row], "cb1[{row}] is not tinted");
        }
    }

    #[test]
    fn camera_underwater_swaps_the_underwater_colour_into_cb1_0() {
        let mut frame = frame_fixture();
        frame.time_water.z = 1.0;
        let environment = WaterEnvironment::new(other_water_far_field());
        let cb1 = pixel_cb1(&ocean_material(), &frame, &environment, false);
        assert_close(cb1[0].truncate(), gamma([100, 110, 120]), "cb1[0].xyz");
        assert_eq!(cb1[0].w, 10000.0, "the sparkle power stays");
    }

    #[test]
    fn exterior_sky_gradient_is_horizon_lower_upper_untinted() {
        let environment = WaterEnvironment {
            tint: Vec3::splat(0.5),
            ..WaterEnvironment::new(other_water_far_field())
        };
        let frame = frame_fixture();
        let cb1 = pixel_cb1(&ocean_material(), &frame, &environment, false);
        assert_eq!(cb1[3], frame.sky_horizon.truncate().extend(0.0));
        assert_eq!(cb1[4], frame.sky_ground.truncate().extend(0.0));
        assert_eq!(cb1[5], frame.sky_zenith.truncate().extend(0.0));
    }

    #[test]
    fn interior_sky_gradient_is_the_tinted_reflection_colour_three_times() {
        let tint = Vec3::new(0.5, 1.0, 2.0);
        let environment = WaterEnvironment {
            tint,
            ..WaterEnvironment::new(other_water_far_field())
        };
        let cb1 = pixel_cb1(&ocean_material(), &frame_fixture(), &environment, true);
        let expected = gamma([50, 60, 70]) * tint;
        for (row, register) in cb1.iter().enumerate().take(6).skip(3) {
            assert_close(register.truncate(), expected, &format!("cb1[{row}]"));
            assert_eq!(register.w, 0.0);
        }
    }

    // --- cb0 ------------------------------------------------

    #[test]
    fn screen_registers_use_the_target_size_and_unit_resolution_scale() {
        let cb0 = frame_constants(&frame_fixture()).cb0;
        assert_eq!(cb0[0], Vec4::new(1.0 / 1920.0, 1.0 / 1080.0, 1.0, 1.0));
        assert_eq!(
            cb0[1],
            Vec4::new(1.0 - 1.0 / 1920.0, 1.0 - 1.0 / 1080.0, 0.0, 0.0)
        );
    }

    #[test]
    fn sun_registers_hold_direction_daylight_colour_and_the_scatter_exponent() {
        let cb0 = frame_constants(&frame_fixture()).cb0;
        assert_eq!(cb0[2], Vec4::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(cb0[3], Vec4::new(3.0, 2.0, 1.0, 8.0));
    }

    #[test]
    fn the_sun_colour_register_drops_the_sunlight_scale_the_frame_folded_in() {
        let mut frame = frame_fixture();
        frame.sun_illuminance = Vec4::new(3.0, 2.0, 1.0, 4.0);
        assert_eq!(
            frame_constants(&frame).cb0[3],
            Vec4::new(0.75, 0.5, 0.25, 8.0)
        );
    }

    #[test]
    fn the_daylight_register_carries_the_suns_alpha_whatever_lights_the_scene() {
        let mut frame = frame_fixture();
        frame.daylight.x = 0.25;
        assert_eq!(frame_constants(&frame).cb0[2].w, 0.25);
        // At night the moon is the light: the direction follows it, the daylight stays 0.
        frame.to_sun = Vec4::new(0.0, 0.5, 0.0, 0.99);
        frame.daylight.x = 0.0;
        let cb0 = frame_constants(&frame).cb0;
        assert_eq!(cb0[2].w, 0.0);
        assert_eq!(
            cb0[2].truncate(),
            axes_from_engine(Vec3::new(0.0, 0.5, 0.0))
        );
    }

    #[test]
    fn ssr_blend_registers_are_the_ini_defaults() {
        let cb0 = frame_constants(&frame_fixture()).cb0;
        assert_eq!(cb0[7], Vec4::new(0.9, 0.2, 0.0, 0.0));
    }

    // --- cb12: fog ------------------------------------------

    #[test]
    fn fog_range_registers_are_in_bethesda_units_and_an_unset_height_band_takes_the_unset_pair() {
        let cb12 = frame_constants(&frame_fixture()).cb12;
        // Near 10 / far 110 engine units = 100 / 1100 Bethesda units.
        let (near, far) = (100.0, 1100.0);
        // The near band starts at -64 and spans 128 engine units: -640 to 640 Bethesda units.
        let (near_bottom, near_top) = (-640.0, 640.0);
        let (unset_near, unset_far) = UNSET_FOG_RANGE;
        let unset_scale = 1.0 / (unset_far - unset_near);
        let unset_offset = unset_near / (unset_far - unset_near);
        let near_height_scale = 1.0 / (near_top - near_bottom);
        let near_height_offset = near_bottom / (near_top - near_bottom);
        assert_eq!(
            cb12[41],
            Vec4::new(
                1.0 / (far - near),
                near_height_scale,
                near / (far - near),
                near_height_offset
            )
        );
        assert_eq!(
            cb12[46],
            Vec4::new(
                near_height_scale,
                unset_scale,
                near_height_offset,
                unset_offset
            )
        );
    }

    #[test]
    fn fog_colour_registers_carry_the_high_colours_power_max_and_high_density_scale() {
        let cb12 = frame_constants(&frame_fixture()).cb12;
        assert_eq!(cb12[42], Vec4::new(0.5, 0.5, 0.6, 2.0));
        assert_eq!(cb12[43], Vec4::new(0.3, 0.35, 0.4, 0.9));
        assert_eq!(cb12[44], Vec4::new(0.7, 0.7, 0.8, 0.95));
        assert_eq!(cb12[45], Vec4::new(0.45, 0.5, 0.6, 0.0));
    }

    #[test]
    fn disabled_fog_takes_the_unset_range_and_stays_zero_at_every_distance() {
        let mut frame = frame_fixture();
        frame.fog_near_color = Vec4::ZERO;
        frame.fog_far_color = Vec4::ZERO;
        frame.fog_params = Vec4::ZERO;
        let cb12 = frame_constants(&frame).cb12;
        assert!(cb12[42].w > 0.0, "power stays finite");
        for distance in [0.0, 100.0, 1.0e6] {
            let raw = distance * cb12[41].x - cb12[41].z;
            assert!(raw < 0.0, "distance {distance}");
        }
    }

    #[test]
    fn camera_constants_are_in_bethesda_units() {
        let cb12 = frame_constants(&frame_fixture()).cb12;
        assert_eq!(cb12[35].truncate(), Vec3::new(1200.0, 480.0, 350.0));
        assert_eq!(cb12[14], Vec4::Z);
        assert_eq!((cb12[0], cb12[1], cb12[2]), (Vec4::X, Vec4::Y, Vec4::Z));
    }

    /// The shader's depth reconstruction: homogeneous `cb12[20..=23]` times
    /// `(ndc.xy, depth, 1)`, then the perspective divide.
    fn reconstruct(frame: &WaterFrameConstants, ndc: Vec4) -> Vec3 {
        let (first, ndc) = if ndc.z <= DEPTH_RANGE_SPLIT {
            (
                DEPTH_NEAR_RANGE_REGISTER,
                Vec4::new(ndc.x, ndc.y, ndc.z * NEAR_RANGE_SCALE, 1.0),
            )
        } else {
            (
                DEPTH_FAR_RANGE_REGISTER,
                Vec4::new(
                    ndc.x,
                    ndc.y,
                    ndc.z * FAR_RANGE_SCALE - DEPTH_RANGE_SPLIT,
                    1.0,
                ),
            )
        };
        let homogeneous = Vec4::new(
            frame.cb12[first].dot(ndc),
            frame.cb12[first + 1].dot(ndc),
            frame.cb12[first + 2].dot(ndc),
            frame.cb12[first + 3].dot(ndc),
        );
        homogeneous.truncate() / homogeneous.w
    }

    #[test]
    fn depth_reconstruction_yields_camera_relative_bethesda_positions() {
        let uniform = frame_fixture();
        let constants = frame_constants(&uniform);
        let camera = uniform.camera_pos.truncate();
        for point in [
            Vec3::new(100.0, 20.0, -90.0),
            Vec3::new(140.0, 30.0, -60.0),
            Vec3::new(121.0, 33.0, -52.0),
        ] {
            let clip = uniform.view_proj * point.extend(1.0);
            let ndc = Vec4::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w, 1.0);
            let expected = position_from_engine(point - camera);
            let actual = reconstruct(&constants, ndc);
            assert!(
                actual.abs_diff_eq(expected, 1e-2 * expected.length().max(1.0)),
                "{point:?}: expected {expected:?}, got {actual:?}"
            );
        }
    }

    // --- cb2 ------------------------------------------------

    #[test]
    fn geometry_rows_map_a_local_vertex_to_camera_relative_bethesda_and_clip_space() {
        let world = Mat4::from_translation(Vec3::new(130.0, 10.0, -70.0));
        let inputs = geometry_fixture(world);
        let (vs_cb2, _) = geometry_registers(&inputs);
        let local = Vec3::new(2.0, 0.0, -3.0);
        let local4 = local.extend(1.0);
        let relative = Vec3::new(
            vs_cb2[0].dot(local4),
            vs_cb2[1].dot(local4),
            vs_cb2[2].dot(local4),
        );
        let world_point = world.transform_point3(local);
        assert!(relative.abs_diff_eq(position_from_engine(world_point - inputs.camera), 1e-3));
        let clip = Vec4::new(
            vs_cb2[3].dot(local4),
            vs_cb2[4].dot(local4),
            vs_cb2[5].dot(local4),
            vs_cb2[6].dot(local4),
        );
        assert!(clip.abs_diff_eq(inputs.view_projection * world_point.extend(1.0), 1e-3));
    }

    #[test]
    fn flat_plane_has_an_upward_bethesda_normal_and_the_camera_height_above_it() {
        let inputs = geometry_fixture(Mat4::from_translation(Vec3::new(130.0, 10.0, -70.0)));
        let (_, ps_cb2) = geometry_registers(&inputs);
        assert!(ps_cb2[4].truncate().abs_diff_eq(Vec3::Z, 1e-6));
        // Camera y 35 over a plane at y 10: 25 engine units, 250 Bethesda units.
        assert!((ps_cb2[4].w - 250.0).abs() < 1e-2);
    }

    #[test]
    fn camera_below_the_plane_has_a_negative_height() {
        let inputs = geometry_fixture(Mat4::from_translation(Vec3::new(0.0, 50.0, 0.0)));
        let (_, ps_cb2) = geometry_registers(&inputs);
        assert!(ps_cb2[4].w < 0.0);
    }

    #[test]
    fn tilted_surface_rotates_the_plane_normal() {
        let world = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let (_, ps_cb2) = geometry_registers(&geometry_fixture(world));
        // Engine +Y rotated about X by 90 degrees is engine +Z = Bethesda -Y.
        assert!(ps_cb2[4].truncate().abs_diff_eq(Vec3::NEG_Y, 1e-6));
    }

    #[test]
    fn plane_w_is_dot_of_normal_with_camera_minus_dot_of_normal_with_the_plane_point() {
        let world = Mat4::from_rotation_translation(
            glam::Quat::from_rotation_x(0.3),
            Vec3::new(130.0, 10.0, -70.0),
        );
        let inputs = geometry_fixture(world);
        let (_, ps_cb2) = geometry_registers(&inputs);
        let normal = ps_cb2[4].truncate();
        let expected = normal.dot(position_from_engine(inputs.camera))
            - normal.dot(position_from_engine(inputs.plane_point));
        assert!((ps_cb2[4].w - expected).abs() < 1e-3);
    }

    #[test]
    fn mesh_uv_flag_travels_in_vs_cb2_7() {
        let mut inputs = geometry_fixture(Mat4::IDENTITY);
        inputs.mesh_uv = true;
        assert_eq!(geometry_registers(&inputs).0[7].x, 1.0);
    }

    // --- scroll ---------------------------------------------

    fn layers_of(material: &WaterMaterial) -> [NoiseLayer; 3] {
        material.layers
    }

    #[test]
    fn a_step_moves_each_layer_by_sin_cos_of_its_wind_direction_in_degrees() {
        let layers = layers_of(&ocean_material());
        let mut scroll = NormalScroll::default();
        scroll.step(&layers, 10.0);
        for (offset, layer) in scroll.offsets().iter().zip(&layers) {
            let theta = layer.wind_direction_degrees.to_radians();
            let distance = 10.0 * layer.wind_speed;
            assert!((offset.x - wrap_scroll(theta.sin() * distance)).abs() < 1e-5);
            assert!((offset.y - wrap_scroll(theta.cos() * distance)).abs() < 1e-5);
        }
    }

    #[test]
    fn scroll_wraps_above_one_with_fmod_and_below_zero_with_fmod_plus_one() {
        let mut layers = layers_of(&ocean_material());
        layers[0].wind_direction_degrees = 90.0;
        layers[0].wind_speed = 1.0;
        layers[1].wind_direction_degrees = 270.0;
        layers[1].wind_speed = 1.0;
        let mut scroll = NormalScroll::default();
        scroll.step(&layers, 1.25);
        let [first, second, _] = scroll.offsets();
        assert!((first.x - 0.25).abs() < 1e-6, "1.25 -> fmod 0.25");
        assert!((second.x - 0.75).abs() < 1e-6, "-1.25 -> fmod -0.25 + 1");
    }

    #[test]
    fn scroll_keeps_an_offset_of_exactly_one() {
        assert_eq!(wrap_scroll(1.0), 1.0);
        assert_eq!(wrap_scroll(0.0), 0.0);
        assert_eq!(wrap_scroll(-1.0), 1.0);
    }

    #[test]
    fn scroll_advances_by_the_clock_delta_once_per_frame() {
        let layers = layers_of(&ocean_material());
        let mut scroll = NormalScroll::default();
        scroll.advance_to(&layers, 100.0);
        assert_eq!(
            scroll.offsets(),
            [Vec2::ZERO; 3],
            "the first call starts the clock"
        );
        scroll.advance_to(&layers, 102.0);
        let after_two_seconds = scroll.offsets();
        scroll.advance_to(&layers, 102.0);
        assert_eq!(scroll.offsets(), after_two_seconds, "same frame, no change");
        let mut reference = NormalScroll::default();
        reference.step(&layers, 2.0);
        assert_eq!(after_two_seconds, reference.offsets());
    }

    #[test]
    fn the_clock_wrap_does_not_make_the_scroll_jump() {
        let layers = layers_of(&ocean_material());
        let mut scroll = NormalScroll::default();
        scroll.advance_to(&layers, TIME_WRAP_SECONDS - 0.5);
        scroll.advance_to(&layers, 0.5);
        let mut reference = NormalScroll::default();
        reference.step(&layers, 1.0);
        assert_eq!(scroll.offsets(), reference.offsets());
    }

    #[test]
    fn vertex_cb1_holds_the_scroll_offsets_and_the_raw_uv_scales() {
        let mut material = ocean_material();
        material.advance_scroll(0.0);
        material.advance_scroll(30.0);
        let cb1 = vertex_cb1(&material.scroll, &material.layers);
        let [first, second, third] = material.scroll.offsets();
        assert_eq!(cb1[0], Vec4::new(first.x, first.y, second.x, second.y));
        assert_eq!(cb1[1], Vec4::new(third.x, third.y, 0.0, 0.0));
        assert_eq!(cb1[2], Vec4::new(8000.0, 500.0, 200.0, 0.0));
    }

    #[test]
    fn draw_constants_assemble_every_stage() {
        let frame = frame_fixture();
        let mut material = ocean_material();
        material.advance_scroll(5.0);
        let environment = WaterEnvironment::new(material.far_field());
        let draw = draw_constants(
            &material,
            &frame,
            &geometry_fixture(Mat4::IDENTITY),
            &environment,
        );
        assert_eq!(draw.ps_cb1[3], frame.sky_horizon.truncate().extend(0.0));
        assert_eq!(draw.vs_cb1[2].x, 8000.0);
        assert!(draw.ps_cb2[4].truncate().abs_diff_eq(Vec3::Z, 1e-6));
    }
}
