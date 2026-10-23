use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::assets::db::lights::Light;
use crate::assets::db::placed_objects::PlacedObject;
use crate::rendering::gpu_slab::SlabElement;
use crate::rendering::shadows::lamps::source::{LampShadowSpec, ShadowType};
use crate::rendering::world_streamer::{SCALE, WorldStreamer, placement_rotation};

/// A placed LIGH reference as the GPU consumes it. A zeroed element
/// (`radius == 0`) is an empty slab slot and lights nothing.
///
/// `color` is linear and pre-animation: `pow(rgb / 255, 2.2) * fade` (see
/// [`fade_for`]); a negative fade (a negative light) gives a negative colour,
/// a zero fade is never placed. Offsets: position 0, radius 12, color 16, flags 28, attenuation 32,
/// `spot_exponent` 44, direction 48, `spot_cos_outer` 60, flicker 64.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuPlacedLight {
    /// Engine space.
    pub position: [f32; 3],
    /// Engine units.
    pub radius: f32,
    pub color: [f32; 3],
    /// `LIGHT_FLAG_*` / `LIGHT_FLAG_REF_*` bits.
    pub flags: u32,
    /// LIGH attenuation constant, scalar and exponent, as stored in the record.
    pub attenuation: [f32; 3],
    /// LIGH falloff exponent, as stored in the record.
    pub spot_exponent: f32,
    /// Unit vector the light shines along, engine space.
    pub direction: [f32; 3],
    /// Cosine of the cone half-angle (see [`outer_cone_cos`]); spotlights only.
    pub spot_cos_outer: f32,
    /// Period, intensity amplitude, movement amplitude, phase in `[0, 1)`.
    /// Stored for later; flicker is not animated.
    pub flicker: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuPlacedLight>() == 80);
const _: () = assert!(std::mem::offset_of!(GpuPlacedLight, flicker) == 64);

impl SlabElement for GpuPlacedLight {
    /// 8192 lights = 640 KiB; an interior holds at most ~1800, an exterior
    /// ring a few thousand.
    const INITIAL_CAPACITY: u32 = 8 * 1024;
    const COMPACTION_FLOOR: u32 = 1024;
}

/// Raw LIGH DATA flag bits are carried unchanged in bits 0..=23 of
/// [`GpuPlacedLight::flags`]. Values are Mutagen 0.50.1's `Light.Flag`; the
/// bits the shaders branch on are mirrored in `shaders/local_lights.wgsl`.
pub const LIGHT_FLAG_CAN_BE_CARRIED: u32 = 0x2;
pub const LIGHT_FLAG_FLICKER: u32 = 0x8;
pub const LIGHT_FLAG_OFF_BY_DEFAULT: u32 = 0x20;
pub const LIGHT_FLAG_PULSE: u32 = 0x80;
pub const LIGHT_FLAG_SHADOW_SPOTLIGHT: u32 = 0x400;
pub const LIGHT_FLAG_SHADOW_HEMISPHERE: u32 = 0x800;
pub const LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL: u32 = 0x1000;
pub const LIGHT_FLAG_NON_SHADOW_SPOTLIGHT: u32 = 0x4000;
pub const LIGHT_FLAG_NON_SPECULAR: u32 = 0x8000;
pub const LIGHT_FLAG_ATTENUATION_ONLY: u32 = 0x1_0000;
pub const LIGHT_FLAG_NON_SHADOW_BOX: u32 = 0x2_0000;
pub const LIGHT_FLAG_IGNORE_ROUGHNESS: u32 = 0x4_0000;
pub const LIGHT_FLAG_NO_RIM_LIGHTING: u32 = 0x8_0000;
pub const LIGHT_FLAG_AMBIENT_ONLY: u32 = 0x10_0000;
/// Lights no lighting consumer uses: the cluster cull (`light_cull.wgsl`), the lamp shadow
/// selection and the effect light pick all skip them.
pub const SKIPPED_FLAGS: u32 = LIGHT_FLAG_OFF_BY_DEFAULT | LIGHT_FLAG_AMBIENT_ONLY;
pub const LIGHT_FLAG_SPOT_MASK: u32 = LIGHT_FLAG_SHADOW_SPOTLIGHT | LIGHT_FLAG_NON_SHADOW_SPOTLIGHT;
const SHADOW_TYPE_FLAGS: [(u32, ShadowType); 3] = [
    (LIGHT_FLAG_SHADOW_SPOTLIGHT, ShadowType::Spot),
    (LIGHT_FLAG_SHADOW_HEMISPHERE, ShadowType::Hemisphere),
    (LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL, ShadowType::Omni),
];
const BASE_FLAGS_MASK: u32 = 0x00FF_FFFF;

/// REFR record-flag bits, mapped into bits 24..=27 of the same word.
pub const LIGHT_FLAG_REF_CASTS_SHADOWS: u32 = 1 << 24;
pub const LIGHT_FLAG_REF_NEVER_FADES: u32 = 1 << 25;
pub const LIGHT_FLAG_REF_DOES_NOT_LIGHT_LANDSCAPE: u32 = 1 << 26;
pub const LIGHT_FLAG_REF_DOES_NOT_LIGHT_WATER: u32 = 1 << 27;

const RECORD_FLAG_DOES_NOT_LIGHT_WATER: u32 = 0x100;
const RECORD_FLAG_CASTS_SHADOWS: u32 = 0x200;
const RECORD_FLAG_NEVER_FADES: u32 = 0x1_0000;
const RECORD_FLAG_DOES_NOT_LIGHT_LANDSCAPE: u32 = 0x2_0000;

const RECORD_TO_LIGHT_FLAGS: [(u32, u32); 4] = [
    (RECORD_FLAG_CASTS_SHADOWS, LIGHT_FLAG_REF_CASTS_SHADOWS),
    (RECORD_FLAG_NEVER_FADES, LIGHT_FLAG_REF_NEVER_FADES),
    (
        RECORD_FLAG_DOES_NOT_LIGHT_LANDSCAPE,
        LIGHT_FLAG_REF_DOES_NOT_LIGHT_LANDSCAPE,
    ),
    (
        RECORD_FLAG_DOES_NOT_LIGHT_WATER,
        LIGHT_FLAG_REF_DOES_NOT_LIGHT_WATER,
    ),
];

/// Values used where the light data is unknown (a database converted before
/// the installer extracted a column, or a NULL). They are the modal vanilla
/// values: exponent 2 on 651 of 801 light records, scalar 1 on 653, constant 0
/// on 795, FOV 90 on 701, falloff exponent 1 on 794, fade 1 on 582.
mod defaults {
    pub const CONSTANT: f32 = 0.0;
    pub const SCALAR: f32 = 1.0;
    pub const EXPONENT: f32 = 2.0;
    pub const FOV_DEGREES: f32 = 90.0;
    pub const FALLOFF_EXPONENT: f32 = 1.0;
    pub const FADE: f32 = 1.0;
    pub const FLICKER: f32 = 0.0;
    /// The Creation Kit's default near clip, also the modal LIGH value (400 of
    /// 801 records).
    pub const NEAR_CLIP: f32 = 7.2174;
}

const COLOR_GAMMA: f32 = 2.2;

/// Direction a light shines along in its local frame, engine space: Gamebryo
/// documents a light's direction as model (1, 0, 0), and with
/// `placement_rotation` +X turns most placed shadow spotlights downwards.
const LOCAL_FORWARD: Vec3 = Vec3::X;

const MIN_FOV_DEGREES: f32 = 0.0;
const MAX_FOV_DEGREES: f32 = 180.0;

#[derive(Debug, thiserror::Error)]
pub enum LightBaseError {
    #[error("no radius")]
    MissingRadius,
    #[error("no colour")]
    MissingColor,
    #[error("no flags")]
    MissingFlags,
    #[error("colour {0:#x} is not a packed 24-bit RGB value")]
    InvalidColor(i64),
    #[error("flags {0:#x} use bits outside the 24 base-flag bits")]
    FlagsOutOfRange(i64),
    #[error("flags {0:#x} combine several shadow light types")]
    AmbiguousShadowType(u32),
}

/// A placed light and, for shadow-type lights, its shadow data.
pub struct PlacedLight {
    pub gpu: GpuPlacedLight,
    pub shadow: Option<LampShadowSpec>,
}

/// What placing a reference yields when it does not fail.
pub enum LightPlacement {
    Lit(PlacedLight),
    /// The combined fade is exactly zero: the reference is placed switched off.
    FadedOut,
}

#[derive(Debug, thiserror::Error)]
pub enum PlacedLightError {
    #[error("effective radius {0} is not a positive finite number")]
    NonPositiveRadius(f32),
    #[error("effective FOV {0} degrees is outside (0, 180]")]
    InvalidFov(f32),
    #[error("effective fade {0} is not a finite number")]
    NonFiniteFade(f32),
    #[error("shadow near clip {0} is not a positive finite number")]
    InvalidNearClip(f32),
    /// The Creation Kit documents that such a light disappears.
    #[error("shadow near clip {near} reaches the light radius {radius}")]
    NearClipBeyondRadius { near: f32, radius: f32 },
}

/// Where and how a light is placed: what a placed reference or an attachment point contributes
/// on top of the light record.
struct Anchor<'a> {
    position: Vec3,
    direction: Vec3,
    scale: f32,
    radius_delta: Option<f64>,
    fade_delta: Option<f64>,
    fov_delta: Option<f64>,
    near_clip_override: Option<f64>,
    depth_bias: Option<f64>,
    record_flags: Option<i64>,
    phase_key: Option<&'a str>,
}

/// The ref-independent half of a placed light, validated once from its LIGH
/// row. Missing optional light data is replaced by [`defaults`] here.
pub struct LightBase {
    radius_beth: f32,
    rgb_linear: Vec3,
    fade: f32,
    flags: u32,
    attenuation: [f32; 3],
    spot_exponent: f32,
    fov_degrees: f32,
    flicker: [f32; 3],
    shadow_type: Option<ShadowType>,
    near_clip_beth: f32,
}

impl LightBase {
    pub fn from_row(light: &Light) -> Result<Self, LightBaseError> {
        let radius_beth = light.radius.ok_or(LightBaseError::MissingRadius)? as f32;
        let packed_color = light.color.ok_or(LightBaseError::MissingColor)?;
        let raw_flags = light.flags.ok_or(LightBaseError::MissingFlags)?;
        let flags = u32::try_from(raw_flags)
            .ok()
            .filter(|flags| flags & !BASE_FLAGS_MASK == 0)
            .ok_or(LightBaseError::FlagsOutOfRange(raw_flags))?;
        let optional = |value: Option<f64>, default: f32| value.map_or(default, |v| v as f32);

        Ok(Self {
            radius_beth,
            rgb_linear: unpack_linear_rgb(packed_color)?,
            fade: optional(light.fade_value, defaults::FADE),
            flags,
            attenuation: [
                optional(light.constant, defaults::CONSTANT),
                optional(light.scalar, defaults::SCALAR),
                optional(light.exponent, defaults::EXPONENT),
            ],
            spot_exponent: optional(light.falloff_exponent, defaults::FALLOFF_EXPONENT),
            fov_degrees: optional(light.fov, defaults::FOV_DEGREES),
            shadow_type: shadow_type(flags)?,
            near_clip_beth: optional(light.near_clip, defaults::NEAR_CLIP),
            flicker: [
                optional(light.flicker_period, defaults::FLICKER),
                optional(light.flicker_intensity_amplitude, defaults::FLICKER),
                optional(light.flicker_movement_amplitude, defaults::FLICKER),
            ],
        })
    }

    pub fn place(&self, reference: &PlacedObject) -> Result<LightPlacement, PlacedLightError> {
        self.place_at(&Anchor {
            position: WorldStreamer::beth_to_engine(
                reference.position_x as f32,
                reference.position_y as f32,
                reference.position_z as f32,
            ),
            direction: (placement_rotation(reference) * LOCAL_FORWARD.extend(0.0))
                .truncate()
                .normalize(),
            scale: reference.scale as f32,
            radius_delta: reference.radius_delta,
            fade_delta: reference.light_fade_pm,
            fov_delta: reference.light_fov_pm,
            near_clip_override: reference.light_near_clip,
            depth_bias: reference.light_shadow_depth_bias,
            record_flags: reference.record_flags,
            phase_key: reference.form_key.as_deref(),
        })
    }

    /// Places the light at an attachment point (an addon node) of a placed model: no ref data,
    /// so the record's own radius, fade, FOV and near clip apply. `transform` is engine space
    /// <- attachment; its translation is the position, its +X the direction, its mean axis
    /// scale the ref scale. `phase_key` seeds the flicker phase.
    pub fn place_attached(
        &self,
        transform: &Mat4,
        phase_key: &str,
    ) -> Result<LightPlacement, PlacedLightError> {
        let scale = (transform.x_axis.truncate().length()
            + transform.y_axis.truncate().length()
            + transform.z_axis.truncate().length())
            / 3.0;
        self.place_at(&Anchor {
            position: transform.w_axis.truncate(),
            direction: transform
                .transform_vector3(LOCAL_FORWARD)
                .try_normalize()
                .unwrap_or(LOCAL_FORWARD),
            scale,
            radius_delta: None,
            fade_delta: None,
            fov_delta: None,
            near_clip_override: None,
            depth_bias: None,
            record_flags: None,
            phase_key: Some(phase_key),
        })
    }

    fn place_at(&self, anchor: &Anchor) -> Result<LightPlacement, PlacedLightError> {
        let radius = radius_for(self.radius_beth, anchor.radius_delta, anchor.scale);
        if !(radius.is_finite() && radius > 0.0) {
            return Err(PlacedLightError::NonPositiveRadius(radius));
        }
        let fade = fade_for(self.fade, anchor.fade_delta);
        if !fade.is_finite() {
            return Err(PlacedLightError::NonFiniteFade(fade));
        }
        if fade == 0.0 {
            return Ok(LightPlacement::FadedOut);
        }
        let spot_cos_outer = if self.flags & LIGHT_FLAG_SPOT_MASK != 0 {
            outer_cone_cos(self.fov_degrees + anchor.fov_delta.unwrap_or(0.0) as f32)?
        } else {
            OMNI_CONE_COS
        };

        let color = self.rgb_linear * fade;
        let [period, intensity_amplitude, movement_amplitude] = self.flicker;
        let shadow = self
            .shadow_type
            .map(|shadow_type| self.shadow_spec(shadow_type, anchor, radius))
            .transpose()?;

        let gpu = GpuPlacedLight {
            position: anchor.position.to_array(),
            radius,
            color: color.to_array(),
            flags: self.flags | reference_flag_bits(anchor.record_flags),
            attenuation: self.attenuation,
            spot_exponent: self.spot_exponent,
            direction: anchor.direction.to_array(),
            spot_cos_outer,
            flicker: [
                period,
                intensity_amplitude,
                movement_amplitude,
                flicker_phase(anchor.phase_key),
            ],
        };
        Ok(LightPlacement::Lit(PlacedLight { gpu, shadow }))
    }

    /// The ref's XLIG near clip replaces the base one when non-zero; like the
    /// radius it scales with the ref.
    fn shadow_spec(
        &self,
        shadow_type: ShadowType,
        anchor: &Anchor,
        radius: f32,
    ) -> Result<LampShadowSpec, PlacedLightError> {
        let near_clip_beth = match anchor.near_clip_override {
            Some(near) if near != 0.0 => near as f32,
            Some(_) | None => self.near_clip_beth,
        };
        let near_clip = near_clip_beth * SCALE * anchor.scale;
        if !(near_clip.is_finite() && near_clip > 0.0) {
            return Err(PlacedLightError::InvalidNearClip(near_clip));
        }
        if near_clip >= radius {
            return Err(PlacedLightError::NearClipBeyondRadius {
                near: near_clip,
                radius,
            });
        }
        Ok(LampShadowSpec {
            shadow_type,
            near_clip,
            depth_bias: anchor.depth_bias.map(|bias| bias as f32),
        })
    }
}

/// At most one shadow light type may be set.
fn shadow_type(flags: u32) -> Result<Option<ShadowType>, LightBaseError> {
    let mut types = SHADOW_TYPE_FLAGS
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|&(_, shadow_type)| shadow_type);
    match (types.next(), types.next()) {
        (first, None) => Ok(first),
        (_, Some(_)) => Err(LightBaseError::AmbiguousShadowType(flags)),
    }
}

/// `pow(channel / 255, 2.2)` per channel of a colour packed as `R << 16 | G << 8 | B`.
fn unpack_linear_rgb(packed: i64) -> Result<Vec3, LightBaseError> {
    let packed = u32::try_from(packed)
        .ok()
        .filter(|packed| packed >> 24 == 0)
        .ok_or(LightBaseError::InvalidColor(packed))?;
    let channel = |shift: u32| ((packed >> shift) & 0xFF) as f32 / 255.0;
    Ok(Vec3::new(channel(16), channel(8), channel(0)).powf(COLOR_GAMMA))
}

/// The ref's XLIG fade is a delta on the base FNAM fade value. FO4 expresses
/// negative lights with a negative fade.
fn fade_for(base_fade: f32, ref_fade_delta: Option<f64>) -> f32 {
    base_fade + ref_fade_delta.unwrap_or(0.0) as f32
}

/// The ref's XRDS is a signed delta on the base radius; the sum is converted
/// to engine units and multiplied by the ref scale.
fn radius_for(base_radius_beth: f32, radius_delta_beth: Option<f64>, ref_scale: f32) -> f32 {
    (base_radius_beth + radius_delta_beth.unwrap_or(0.0) as f32) * SCALE * ref_scale
}

/// Only spotlights have a cone; every other light stores the cosine of a
/// 180 degree half-angle, which no consumer reads.
const OMNI_CONE_COS: f32 = -1.0;

/// LIGH FOV is the full cone angle in degrees (the ref's XLIG FOV is added to
/// it); the shader wants the cosine of the half-angle:
/// `cos(radians(fov) / 2)`. 180 degrees gives 0 (a hemisphere).
fn outer_cone_cos(fov_degrees: f32) -> Result<f32, PlacedLightError> {
    if !(fov_degrees > MIN_FOV_DEGREES && fov_degrees <= MAX_FOV_DEGREES) {
        return Err(PlacedLightError::InvalidFov(fov_degrees));
    }
    Ok((fov_degrees.to_radians() * 0.5).cos())
}

fn reference_flag_bits(record_flags: Option<i64>) -> u32 {
    let record_flags = record_flags.unwrap_or(0) as u32;
    RECORD_TO_LIGHT_FLAGS
        .iter()
        .filter(|(record_bit, _)| record_flags & record_bit != 0)
        .fold(0, |bits, (_, light_bit)| bits | light_bit)
}

/// Deterministic per-reference phase in `[0, 1)` so neighbouring lamps will
/// not flicker in lock-step once flicker is animated.
fn flicker_phase(form_key: Option<&str>) -> f32 {
    let Some(form_key) = form_key else {
        return 0.0;
    };
    let hash = crate::sim::refr_entity_id(form_key).0;
    (hash >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact compositions of them"
)]
mod tests {
    use super::*;

    fn row(radius: f64, color: i64, fade: Option<f64>) -> Light {
        Light {
            form_key: Some("0A:light.esm".to_owned()),
            editor_id: None,
            model_path: None,
            radius: Some(radius),
            color: Some(color),
            flags: Some(0),
            fade_value: fade,
            falloff_exponent: None,
            is_marker: false,
            constant: None,
            scalar: None,
            exponent: None,
            fov: None,
            near_clip: None,
            flicker_period: None,
            flicker_intensity_amplitude: None,
            flicker_movement_amplitude: None,
            gobo: None,
            lens: None,
            god_rays: None,
            god_rays_near_clip: None,
        }
    }

    fn lit(placement: LightPlacement) -> GpuPlacedLight {
        match placement {
            LightPlacement::Lit(placed) => placed.gpu,
            LightPlacement::FadedOut => panic!("light is faded out"),
        }
    }

    #[test]
    fn an_attached_light_takes_position_direction_and_scale_from_the_transform() {
        let base = LightBase::from_row(&row(100.0, 0x00FF_FFFF, None)).unwrap();
        let transform = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)
            * Mat4::from_scale(Vec3::splat(2.0));

        let light = lit(base.place_attached(&transform, "key").unwrap());

        assert_eq!(light.position, [1.0, 2.0, 3.0]);
        assert!(
            (light.radius - 20.0).abs() < 1e-4,
            "radius {}",
            light.radius
        );
        let direction = Vec3::from(light.direction);
        assert!(
            (direction - Vec3::Y).length() < 1e-5,
            "direction {direction}"
        );
    }

    #[test]
    fn an_attached_light_uses_the_record_fade_and_a_zero_fade_switches_it_off() {
        let faded = LightBase::from_row(&row(100.0, 0x00FF_FFFF, Some(0.0))).unwrap();
        let half = LightBase::from_row(&row(100.0, 0x00FF_FFFF, Some(0.5))).unwrap();

        assert!(matches!(
            faded.place_attached(&Mat4::IDENTITY, "k").unwrap(),
            LightPlacement::FadedOut
        ));
        assert_eq!(
            lit(half.place_attached(&Mat4::IDENTITY, "k").unwrap()).color,
            [0.5; 3]
        );
    }

    #[test]
    fn the_flicker_phase_follows_the_key() {
        let base = LightBase::from_row(&row(100.0, 0x00FF_FFFF, None)).unwrap();
        let phase = |key| lit(base.place_attached(&Mat4::IDENTITY, key).unwrap()).flicker[3];

        assert_eq!(phase("a"), phase("a"));
        assert_ne!(phase("a"), phase("b"));
        assert!((0.0..1.0).contains(&phase("a")));
    }
}
