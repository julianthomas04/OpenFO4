use glam::Vec3;
use serde::Deserialize;

use super::ambient::{AMBIENT_CUBE_FACES, AmbientCube};
use super::fog::measured::INTERIOR_COLOUR_FACTORS;
use super::fog::{Fog, FogColors, FogParams, HeightBands, SunlightTerm};
use crate::rendering::world_streamer::SCALE;

#[derive(Debug, thiserror::Error)]
pub enum CellLightingError {
    #[error("no lighting data")]
    Absent,
    #[error("malformed lighting JSON: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("lighting field `{field}` is missing")]
    MissingField { field: &'static str },
    #[error("lighting field `{field}` is not finite")]
    NonFinite { field: &'static str },
    #[error("fog range is not increasing (near {near}, far {far} Bethesda units)")]
    InvertedFogRange { near: f32, far: f32 },
    #[error("lighting field `{field}` is negative")]
    NegativeFogScale { field: &'static str },
}

impl CellLightingError {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Malformed(_) => "malformed",
            Self::MissingField { .. } => "missing field",
            Self::NonFinite { .. } => "non-finite field",
            Self::InvertedFogRange { .. } => "inverted fog range",
            Self::NegativeFogScale { .. } => "negative fog scale",
        }
    }
}

/// Bit layout of the `Inherits` field of a cell's lighting; a set bit takes
/// that field from the cell's lighting template.
#[derive(Debug, Copy, Clone)]
pub struct Inherits(u32);

impl Inherits {
    pub const AMBIENT_COLOR: u32 = 1 << 0;
    pub const DIRECTIONAL_COLOR: u32 = 1 << 1;
    pub const FOG_COLOR: u32 = 1 << 2;
    pub const FOG_NEAR: u32 = 1 << 3;
    pub const FOG_FAR: u32 = 1 << 4;
    pub const DIRECTIONAL_ROTATION: u32 = 1 << 5;
    pub const DIRECTIONAL_FADE: u32 = 1 << 6;
    pub const CLIP_DISTANCE: u32 = 1 << 7;
    pub const FOG_POWER: u32 = 1 << 8;
    pub const FOG_MAX: u32 = 1 << 9;
    pub const LIGHT_FADE_DISTANCES: u32 = 1 << 10;
    const ALL: u32 = (1 << 11) - 1;

    fn contains(self, bit: u32) -> bool {
        self.0 & bit != 0
    }

    fn any(self) -> bool {
        self.contains(Self::ALL)
    }
}

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RawColor {
    r: u8,
    g: u8,
    b: u8,
}

impl RawColor {
    fn is_black(self) -> bool {
        self.r == 0 && self.g == 0 && self.b == 0
    }

    pub(super) fn linear(self) -> Vec3 {
        self.decoded(StoredColourEncoding::Srgb)
    }

    pub(super) fn decoded(self, encoding: StoredColourEncoding) -> Vec3 {
        encoding.decode(self.unit())
    }

    /// Each channel as stored, scaled to 0 to 1.
    pub(super) fn unit(self) -> Vec3 {
        Vec3::new(f32::from(self.r), f32::from(self.g), f32::from(self.b)) / 255.0
    }

    /// Decoded with gamma 2.2, the space the fog blends its colours in.
    pub(super) fn gamma_linear(self) -> Vec3 {
        self.decoded(StoredColourEncoding::Gamma22)
    }
}

/// How a colour stored as 0 to 1 per channel becomes linear light, and back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredColourEncoding {
    Srgb,
    Gamma22,
}

impl StoredColourEncoding {
    pub(crate) fn decode(self, stored: Vec3) -> Vec3 {
        match self {
            Self::Srgb => Vec3::from_array(stored.to_array().map(srgb_unit_to_linear)),
            Self::Gamma22 => gamma_decode(stored),
        }
    }

    /// The exact inverse of [`Self::decode`]; negative channels are 0.
    pub(crate) fn encode(self, linear: Vec3) -> Vec3 {
        let linear = linear.max(Vec3::ZERO);
        match self {
            Self::Srgb => Vec3::from_array(linear.to_array().map(linear_to_srgb_unit)),
            Self::Gamma22 => linear.powf(DISPLAY_GAMMA.recip()),
        }
    }
}

const DISPLAY_GAMMA: f32 = 2.2;

/// A colour stored as 0 to 1 per channel, decoded with gamma 2.2.
pub(crate) fn gamma_decode(encoded: Vec3) -> Vec3 {
    encoded.powf(DISPLAY_GAMMA)
}

pub(crate) fn srgb_to_linear(channel: u8) -> f32 {
    srgb_unit_to_linear(f32::from(channel) / 255.0)
}

/// One sRGB channel stored as 0 to 1, as linear light.
pub(crate) fn srgb_unit_to_linear(encoded: f32) -> f32 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_unit(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(2.4_f32.recip()) - 0.055
    }
}

/// A colour stored as 0 to 255 per channel, as the linear light the lighting adds up.
pub(crate) fn srgb8_to_linear([red, green, blue]: [u8; 3]) -> Vec3 {
    Vec3::new(
        srgb_to_linear(red),
        srgb_to_linear(green),
        srgb_to_linear(blue),
    )
}

/// Rec. 709 luma weights: how much each linear channel counts towards how bright a colour looks.
const LUMINANCE_WEIGHTS: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// How bright the linear colour `light` looks: 1 for white at full strength.
pub(crate) fn luminance(light: Vec3) -> f32 {
    light.dot(LUMINANCE_WEIGHTS)
}

/// Bethesda Z is up and Y is forward; engine Y is up and -Z is forward.
/// Faces go in as +X, -X, +Y, -Y, +Z, -Z and come out in engine order.
pub(super) fn bethesda_faces_to_engine_order<T: Copy>(
    faces: [T; AMBIENT_CUBE_FACES],
) -> [T; AMBIENT_CUBE_FACES] {
    let [x_plus, x_minus, y_plus, y_minus, z_plus, z_minus] = faces;
    [x_plus, x_minus, z_plus, z_minus, y_minus, y_plus]
}

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[allow(
    clippy::struct_field_names,
    reason = "field names mirror the serialized ambient-cube record keys"
)]
struct RawAmbientCube {
    directional_x_plus: RawColor,
    directional_x_minus: RawColor,
    directional_y_plus: RawColor,
    directional_y_minus: RawColor,
    directional_z_plus: RawColor,
    directional_z_minus: RawColor,
}

impl RawAmbientCube {
    fn is_black(self) -> bool {
        self.faces_in_engine_order()
            .iter()
            .all(|face| face.is_black())
    }

    fn faces_in_engine_order(self) -> [RawColor; AMBIENT_CUBE_FACES] {
        bethesda_faces_to_engine_order([
            self.directional_x_plus,
            self.directional_x_minus,
            self.directional_y_plus,
            self.directional_y_minus,
            self.directional_z_plus,
            self.directional_z_minus,
        ])
    }

    fn linear(self) -> AmbientCube {
        AmbientCube(self.faces_in_engine_order().map(RawColor::linear))
    }
}

/// One serde shape for both cell lighting (XCLL) and lighting template data;
/// the aliases cover the template's differently named fields. Fields the engine
/// does not use (light fade distances, specular, ambient scale) are ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RawLighting {
    ambient_color: Option<RawColor>,
    directional_color: Option<RawColor>,
    fog_near_color: Option<RawColor>,
    fog_far_color: Option<RawColor>,
    fog_near: Option<f32>,
    fog_far: Option<f32>,
    #[serde(rename = "DirectionalRotationXY")]
    directional_rotation_xy: Option<f32>,
    directional_rotation_z: Option<f32>,
    directional_fade: Option<f32>,
    fog_clip_distance: Option<f32>,
    fog_power: Option<f32>,
    fog_max: Option<f32>,
    #[serde(alias = "DirectionalAmbientColors")]
    ambient_colors: Option<RawAmbientCube>,
    inherits: Option<u32>,
    fog_near_scale: Option<f32>,
    fog_far_scale: Option<f32>,
}

impl RawLighting {
    /// `None`, blank text, `null` and `{}` (exterior cells) are all
    /// [`CellLightingError::Absent`].
    pub fn parse(json: Option<&str>) -> Result<Self, CellLightingError> {
        let json = json
            .filter(|text| !text.trim().is_empty())
            .ok_or(CellLightingError::Absent)?;
        let value: serde_json::Value = serde_json::from_str(json)?;
        let has_content = match &value {
            serde_json::Value::Null => false,
            serde_json::Value::Object(fields) => !fields.is_empty(),
            _ => true,
        };
        if !has_content {
            return Err(CellLightingError::Absent);
        }
        Ok(serde_json::from_value(value)?)
    }

    fn inherits(&self) -> Inherits {
        Inherits(self.inherits.unwrap_or(0))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TemplateGap {
    UnknownTemplate,
    NoData,
    Malformed,
}

impl TemplateGap {
    pub fn label(self) -> &'static str {
        match self {
            Self::UnknownTemplate => "template not in database",
            Self::NoData => "template has no data",
            Self::Malformed => "template data malformed",
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum TemplateSource<'a> {
    Unlinked,
    LinkUnknown,
    Available(&'a RawLighting),
    Unavailable { form_key: &'a str, gap: TemplateGap },
}

impl<'a> TemplateSource<'a> {
    fn raw(self) -> Option<&'a RawLighting> {
        match self {
            Self::Available(raw) => Some(raw),
            Self::Unlinked | Self::LinkUnknown | Self::Unavailable { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateStatus {
    NoInheritance,
    Merged,
    NoTemplateLinked,
    LinkColumnMissing,
    Unavailable { form_key: String, gap: TemplateGap },
}

impl TemplateStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NoInheritance => "no inheritance",
            Self::Merged => "merged with template",
            Self::NoTemplateLinked => "inherits but no template linked",
            Self::LinkColumnMissing => "inherits but database has no template links",
            Self::Unavailable { gap, .. } => gap.label(),
        }
    }

    fn of(inherits: Inherits, source: TemplateSource) -> Self {
        if !inherits.any() {
            return Self::NoInheritance;
        }
        match source {
            TemplateSource::Available(_) => Self::Merged,
            TemplateSource::Unlinked => Self::NoTemplateLinked,
            TemplateSource::LinkUnknown => Self::LinkColumnMissing,
            TemplateSource::Unavailable { form_key, gap } => Self::Unavailable {
                form_key: form_key.to_string(),
                gap,
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub to_light: Vec3,
    pub fade: f32,
}

/// Engine-space interior lighting: linear colours, engine units, unit vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct CellLighting {
    pub ambient_color: Vec3,
    pub ambient_cube: AmbientCube,
    pub directional: Option<DirectionalLight>,
    pub fog: Fog,
    pub clip_distance: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLighting {
    pub lighting: CellLighting,
    pub template: TemplateStatus,
}

/// Merges a cell's lighting with its template per the cell's `Inherits` bits.
/// An unavailable template leaves the cell's own values in place and is
/// reported through [`ResolvedLighting::template`].
pub fn resolve(
    cell: &RawLighting,
    template: TemplateSource,
) -> Result<ResolvedLighting, CellLightingError> {
    let inherits = cell.inherits();
    let merge = Merge {
        cell,
        template: template.raw().filter(|_| inherits.any()),
        inherits,
    };
    Ok(ResolvedLighting {
        lighting: merge.lighting()?,
        template: TemplateStatus::of(inherits, template),
    })
}

struct Merge<'a> {
    cell: &'a RawLighting,
    template: Option<&'a RawLighting>,
    inherits: Inherits,
}

impl Merge<'_> {
    fn lighting(&self) -> Result<CellLighting, CellLightingError> {
        Ok(CellLighting {
            ambient_color: self
                .pick(Inherits::AMBIENT_COLOR, "AmbientColor", |raw| {
                    raw.ambient_color
                })?
                .linear(),
            ambient_cube: self.ambient_cube()?,
            directional: self.directional()?,
            fog: self.fog()?,
            clip_distance: self.clip_distance()?,
        })
    }

    fn pick<T>(
        &self,
        bit: u32,
        field: &'static str,
        get: impl Fn(&RawLighting) -> Option<T>,
    ) -> Result<T, CellLightingError> {
        get(self.source(bit)).ok_or(CellLightingError::MissingField { field })
    }

    /// The record a field under the inherit `bit` comes from.
    fn source(&self, bit: u32) -> &RawLighting {
        match self.template {
            Some(template) if self.inherits.contains(bit) => template,
            _ => self.cell,
        }
    }

    fn pick_finite(
        &self,
        bit: u32,
        field: &'static str,
        get: impl Fn(&RawLighting) -> Option<f32>,
    ) -> Result<f32, CellLightingError> {
        let value = self.pick(bit, field, get)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(CellLightingError::NonFinite { field })
        }
    }

    /// No inherit bit covers the ambient cube: the cell's own cube wins unless
    /// it is entirely black, then the template's applies.
    fn ambient_cube(&self) -> Result<AmbientCube, CellLightingError> {
        let cell_cube = self.cell.ambient_colors;
        let template_cube = self.template.and_then(|template| template.ambient_colors);
        cell_cube
            .filter(|cube| !cube.is_black())
            .or(template_cube)
            .or(cell_cube)
            .map(RawAmbientCube::linear)
            .ok_or(CellLightingError::MissingField {
                field: "AmbientColors",
            })
    }

    fn directional(&self) -> Result<Option<DirectionalLight>, CellLightingError> {
        let color = self
            .pick(Inherits::DIRECTIONAL_COLOR, "DirectionalColor", |raw| {
                raw.directional_color
            })?
            .linear();
        if color == Vec3::ZERO {
            return Ok(None);
        }
        let rotation_xy = self.pick_finite(
            Inherits::DIRECTIONAL_ROTATION,
            "DirectionalRotationXY",
            |raw| raw.directional_rotation_xy,
        )?;
        let rotation_z = self.pick_finite(
            Inherits::DIRECTIONAL_ROTATION,
            "DirectionalRotationZ",
            |raw| raw.directional_rotation_z,
        )?;
        let fade = self.pick_finite(Inherits::DIRECTIONAL_FADE, "DirectionalFade", |raw| {
            raw.directional_fade
        })?;
        Ok(Some(DirectionalLight {
            color,
            to_light: to_light_direction(rotation_xy, rotation_z),
            fade,
        }))
    }

    fn fog(&self) -> Result<Fog, CellLightingError> {
        let near = self.pick_finite(Inherits::FOG_NEAR, "FogNear", |raw| raw.fog_near)?;
        let far = self.pick_finite(Inherits::FOG_FAR, "FogFar", |raw| raw.fog_far)?;
        if near == 0.0 && far == 0.0 {
            return Ok(Fog::Disabled);
        }
        if far <= near {
            return Err(CellLightingError::InvertedFogRange { near, far });
        }
        let scales = FogColorScales::of(self.source(Inherits::FOG_COLOR))?;
        let near_color = self
            .pick(Inherits::FOG_COLOR, "FogNearColor", |raw| {
                raw.fog_near_color
            })?
            .gamma_linear()
            * scales.near;
        let far_color = self
            .pick(Inherits::FOG_COLOR, "FogFarColor", |raw| raw.fog_far_color)?
            .gamma_linear()
            * scales.far;
        Ok(Fog::Enabled(FogParams {
            colors: INTERIOR_COLOUR_FACTORS.apply(FogColors {
                near: near_color,
                far: far_color,
                near_high: near_color,
                far_high: far_color,
            }),
            near: near * SCALE,
            far: far * SCALE,
            power: self.pick_finite(Inherits::FOG_POWER, "FogPower", |raw| raw.fog_power)?,
            max: self.pick_finite(Inherits::FOG_MAX, "FogMax", |raw| raw.fog_max)?,
            height: HeightBands::NONE,
            sunlight: SunlightTerm::NONE,
        }))
    }

    fn clip_distance(&self) -> Result<Option<f32>, CellLightingError> {
        let distance = self.pick_finite(Inherits::CLIP_DISTANCE, "FogClipDistance", |raw| {
            raw.fog_clip_distance
        })?;
        Ok((distance > 0.0).then_some(distance * SCALE))
    }
}

/// The `Fog*Scale` multipliers of the near and far fog colours, which are bytes and so cannot be
/// brighter than 255 on their own.
#[derive(Debug, Copy, Clone, PartialEq)]
struct FogColorScales {
    near: f32,
    far: f32,
}

impl FogColorScales {
    const UNSCALED: Self = Self {
        near: 1.0,
        far: 1.0,
    };

    /// Records that predate the scales store 0 for both, and older databases none: such fog is
    /// unscaled. A single missing scale is 1.
    fn of(raw: &RawLighting) -> Result<Self, CellLightingError> {
        let fields = [
            ("FogNearScale", raw.fog_near_scale),
            ("FogFarScale", raw.fog_far_scale),
        ];
        if fields
            .iter()
            .all(|(_, scale)| scale.is_none_or(|scale| scale == 0.0))
        {
            return Ok(Self::UNSCALED);
        }
        let [near, far] = fields.map(|(field, scale)| {
            let scale = scale.unwrap_or(1.0);
            if !scale.is_finite() {
                Err(CellLightingError::NonFinite { field })
            } else if scale < 0.0 {
                Err(CellLightingError::NegativeFogScale { field })
            } else {
                Ok(scale)
            }
        });
        Ok(Self {
            near: near?,
            far: far?,
        })
    }
}

/// Bethesda's light shines along `Rz(xy) * Ry(z)` applied to +X, with a positive
/// XY angle turning clockwise seen from above; the direction to the light is
/// its negation, converted from (x, y, z-up) to engine (x, z, -y).
fn to_light_direction(rotation_xy_deg: f32, rotation_z_deg: f32) -> Vec3 {
    let (sin_xy, cos_xy) = rotation_xy_deg.to_radians().sin_cos();
    let (sin_z, cos_z) = rotation_z_deg.to_radians().sin_cos();
    Vec3::new(-cos_xy * cos_z, -sin_z, -sin_xy * cos_z)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn color(r: u8, g: u8, b: u8) -> Value {
        json!({"R": r, "G": g, "B": b})
    }

    fn cube(faces: [Value; 6]) -> Value {
        let [xp, xm, yp, ym, zp, zm] = faces;
        json!({
            "DirectionalXPlus": xp, "DirectionalXMinus": xm,
            "DirectionalYPlus": yp, "DirectionalYMinus": ym,
            "DirectionalZPlus": zp, "DirectionalZMinus": zm,
        })
    }

    /// A complete cell lighting record; `overrides` replaces or adds top-level fields.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "json! literals are passed by value"
    )]
    fn record(overrides: Value) -> Value {
        let mut base = json!({
            "AmbientColor": color(255, 255, 255),
            "DirectionalColor": color(255, 255, 255),
            "FogNearColor": color(0, 0, 255),
            "FogFarColor": color(255, 0, 0),
            "FogNear": 100.0,
            "FogFar": 1000.0,
            "DirectionalRotationXY": 0.0,
            "DirectionalRotationZ": 0.0,
            "DirectionalFade": 1.0,
            "FogClipDistance": 4000.0,
            "FogPower": 2.0,
            "FogMax": 0.5,
            "AmbientColors": cube(std::array::from_fn(|_| color(10, 20, 30))),
            "Inherits": 0,
        });
        base.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        base
    }

    fn raw(overrides: Value) -> RawLighting {
        RawLighting::parse(Some(&record(overrides).to_string())).unwrap()
    }

    fn resolved(overrides: Value) -> Result<ResolvedLighting, CellLightingError> {
        resolve(&raw(overrides), TemplateSource::Unlinked)
    }

    fn without(field: &str) -> RawLighting {
        let mut value = record(json!({}));
        value.as_object_mut().unwrap().remove(field);
        RawLighting::parse(Some(&value.to_string())).unwrap()
    }

    #[test]
    fn srgb_endpoints_map_to_zero_and_one() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn srgb_uses_the_linear_segment_below_the_threshold_and_the_power_curve_above() {
        // 10/255 = 0.0392 is below 0.04045: encoded / 12.92.
        assert!((srgb_to_linear(10) - 0.003_035).abs() < 1e-5);
        // Mid grey 128 is 0.2158605 in linear light (standard sRGB table value).
        assert!((srgb_to_linear(128) - 0.215_860).abs() < 1e-4);
    }

    #[test]
    fn a_stored_colour_converts_each_channel_by_itself() {
        let linear = srgb8_to_linear([128, 64, 200]);
        for (channel, expected) in [
            (linear.x, 0.215_860_5),
            (linear.y, 0.051_269_46),
            (linear.z, 0.577_580_4),
        ] {
            assert!((channel - expected).abs() < 1e-6, "{channel} vs {expected}");
        }
    }

    #[test]
    fn no_light_is_no_luminance_and_white_light_at_full_strength_is_one() {
        assert_eq!(luminance(Vec3::ZERO), 0.0);
        assert!((luminance(Vec3::ONE) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn each_channel_counts_by_its_share_of_the_luminance() {
        assert!((luminance(Vec3::X) - 0.2126).abs() < 1e-6);
        assert!((luminance(Vec3::Y) - 0.7152).abs() < 1e-6);
        assert!((luminance(Vec3::Z) - 0.0722).abs() < 1e-6);
    }

    #[test]
    fn the_luminance_of_light_scales_with_the_light() {
        let light = Vec3::new(0.5, 0.25, 1.0);
        assert!((luminance(light * 3.0) - 3.0 * luminance(light)).abs() < 1e-6);
    }

    #[test]
    fn bethesda_faces_are_reordered_into_engine_axes() {
        // Bethesda +Y (forward) is engine -Z and Bethesda +Z (up) is engine +Y.
        let reordered = bethesda_faces_to_engine_order(["x+", "x-", "y+", "y-", "z+", "z-"]);
        assert_eq!(reordered, ["x+", "x-", "z+", "z-", "y-", "y+"]);
    }

    #[test]
    fn missing_blank_null_and_empty_lighting_are_all_absent() {
        for json in [None, Some(""), Some("  \n"), Some("null"), Some("{}")] {
            assert!(
                matches!(RawLighting::parse(json), Err(CellLightingError::Absent)),
                "{json:?}"
            );
        }
    }

    #[test]
    fn malformed_lighting_json_is_reported_as_malformed_not_absent() {
        for json in ["{", "[1, 2", "{\"FogNear\": \"far\"}"] {
            assert!(
                matches!(
                    RawLighting::parse(Some(json)),
                    Err(CellLightingError::Malformed(_))
                ),
                "{json}"
            );
        }
    }

    #[test]
    fn template_data_uses_the_aliased_ambient_cube_key() {
        let template = json!({
            "DirectionalAmbientColors": cube(std::array::from_fn(|_| color(255, 255, 255))),
        });
        let parsed = RawLighting::parse(Some(&template.to_string())).unwrap();
        assert!(parsed.ambient_colors.is_some());
    }

    #[test]
    fn a_complete_cell_resolves_to_engine_units_and_linear_colours() {
        let lighting = resolved(json!({})).unwrap().lighting;
        assert_eq!(lighting.ambient_color, Vec3::ONE);
        let Fog::Enabled(fog) = lighting.fog else {
            panic!("fog should be enabled");
        };
        assert!((fog.near - 10.0).abs() < 1e-4);
        assert!((fog.far - 100.0).abs() < 1e-3);
        assert_eq!((fog.power, fog.max), (2.0, 0.5));
        assert!((lighting.clip_distance.unwrap() - 400.0).abs() < 1e-2);
    }

    #[test]
    fn a_cell_without_a_required_field_names_the_missing_field() {
        for (field, name) in [
            ("AmbientColor", "AmbientColor"),
            ("DirectionalColor", "DirectionalColor"),
            ("FogNear", "FogNear"),
            ("FogFar", "FogFar"),
            ("FogClipDistance", "FogClipDistance"),
            ("AmbientColors", "AmbientColors"),
        ] {
            let result = resolve(&without(field), TemplateSource::Unlinked);
            assert!(
                matches!(&result, Err(CellLightingError::MissingField { field: f }) if *f == name),
                "{field}: {result:?}"
            );
        }
    }

    #[test]
    fn a_fog_range_that_does_not_increase_is_rejected() {
        for (near, far) in [(500.0, 100.0), (100.0, 100.0)] {
            let result = resolved(json!({"FogNear": near, "FogFar": far}));
            assert!(
                matches!(result, Err(CellLightingError::InvertedFogRange { .. })),
                "{near}..{far}"
            );
        }
    }

    #[test]
    fn a_zero_fog_range_disables_fog() {
        let lighting = resolved(json!({"FogNear": 0.0, "FogFar": 0.0}))
            .unwrap()
            .lighting;
        assert_eq!(lighting.fog, Fog::Disabled);
    }

    #[test]
    fn disabled_fog_does_not_need_the_fog_colours() {
        let mut value = record(json!({"FogNear": 0.0, "FogFar": 0.0}));
        value.as_object_mut().unwrap().remove("FogNearColor");
        let parsed = RawLighting::parse(Some(&value.to_string())).unwrap();
        assert!(resolve(&parsed, TemplateSource::Unlinked).is_ok());
    }

    #[test]
    fn a_black_directional_colour_means_no_directional_light() {
        let lighting = resolved(json!({"DirectionalColor": color(0, 0, 0)}))
            .unwrap()
            .lighting;
        assert_eq!(lighting.directional, None);
    }

    #[test]
    fn directional_light_points_back_along_the_rotated_x_axis() {
        // (rotation XY, rotation Z) in degrees -> direction to the light in engine axes.
        let cases = [
            (0.0, 0.0, Vec3::new(-1.0, 0.0, 0.0)),
            (0.0, 90.0, Vec3::new(0.0, -1.0, 0.0)),
            (90.0, 0.0, Vec3::new(0.0, 0.0, -1.0)),
            (180.0, 0.0, Vec3::new(1.0, 0.0, 0.0)),
        ];
        for (xy, z, expected) in cases {
            let lighting =
                resolved(json!({"DirectionalRotationXY": xy, "DirectionalRotationZ": z}))
                    .unwrap()
                    .lighting;
            let to_light = lighting.directional.unwrap().to_light;
            assert!(to_light.abs_diff_eq(expected, 1e-5), "{xy},{z}: {to_light}");
        }
    }

    #[test]
    fn a_non_positive_clip_distance_means_no_clipping() {
        let lighting = resolved(json!({"FogClipDistance": 0.0})).unwrap().lighting;
        assert_eq!(lighting.clip_distance, None);
    }

    #[test]
    fn the_ambient_cube_faces_follow_the_engine_axis_order() {
        let faces = [10, 20, 30, 40, 50, 60].map(|red| color(red, 0, 0));
        let lighting = resolved(json!({"AmbientColors": cube(faces)}))
            .unwrap()
            .lighting;
        let reds: Vec<f32> = lighting.ambient_cube.0.iter().map(|face| face.x).collect();
        // Bethesda x+, x-, y+, y-, z+, z- = 10..60 come out as x+, x-, z+, z-, y-, y+.
        let expected = [10, 20, 50, 60, 40, 30].map(srgb_to_linear);
        for (actual, wanted) in reds.iter().zip(expected) {
            assert!((actual - wanted).abs() < 1e-6, "{reds:?}");
        }
    }

    #[test]
    fn an_inherited_field_is_taken_from_the_template() {
        let template = raw(json!({"AmbientColor": color(0, 0, 0)}));
        let cell = raw(json!({"Inherits": Inherits::AMBIENT_COLOR}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        assert_eq!(merged.lighting.ambient_color, Vec3::ZERO);
        assert_eq!(merged.template, TemplateStatus::Merged);
    }

    #[test]
    fn an_inherited_clip_distance_is_the_templates_even_when_the_cell_has_none() {
        let template = raw(json!({"FogClipDistance": 12_000.0}));
        let cell = raw(json!({"FogClipDistance": 0.0, "Inherits": Inherits::CLIP_DISTANCE}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        assert!((merged.lighting.clip_distance.unwrap() - 1200.0).abs() < 1e-2);
    }

    #[test]
    fn a_field_that_is_not_inherited_stays_the_cells_own() {
        let template = raw(json!({"AmbientColor": color(0, 0, 0), "FogNear": 5.0, "FogFar": 50.0}));
        let cell = raw(json!({"Inherits": Inherits::AMBIENT_COLOR}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        let Fog::Enabled(fog) = merged.lighting.fog else {
            panic!("fog should be enabled");
        };
        assert!((fog.near - 10.0).abs() < 1e-4);
    }

    #[test]
    fn a_template_is_ignored_when_nothing_is_inherited() {
        let template = raw(json!({"AmbientColor": color(0, 0, 0)}));
        let cell = raw(json!({"Inherits": 0}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        assert_eq!(merged.lighting.ambient_color, Vec3::ONE);
        assert_eq!(merged.template, TemplateStatus::NoInheritance);
    }

    #[test]
    fn an_unavailable_template_keeps_the_cells_values_and_reports_why() {
        let cell = raw(json!({"Inherits": Inherits::AMBIENT_COLOR}));
        let source = TemplateSource::Unavailable {
            form_key: "012345:Fallout4.esm",
            gap: TemplateGap::NoData,
        };
        let merged = resolve(&cell, source).unwrap();
        assert_eq!(merged.lighting.ambient_color, Vec3::ONE);
        assert_eq!(
            merged.template,
            TemplateStatus::Unavailable {
                form_key: "012345:Fallout4.esm".to_owned(),
                gap: TemplateGap::NoData
            }
        );
    }

    #[test]
    fn inheriting_without_a_linked_template_is_reported() {
        let cell = raw(json!({"Inherits": Inherits::FOG_NEAR}));
        assert_eq!(
            resolve(&cell, TemplateSource::Unlinked).unwrap().template,
            TemplateStatus::NoTemplateLinked
        );
        assert_eq!(
            resolve(&cell, TemplateSource::LinkUnknown)
                .unwrap()
                .template,
            TemplateStatus::LinkColumnMissing
        );
    }

    #[test]
    fn a_black_cell_ambient_cube_falls_back_to_the_templates() {
        let black = cube(std::array::from_fn(|_| color(0, 0, 0)));
        let white = cube(std::array::from_fn(|_| color(255, 255, 255)));
        let template = raw(json!({"AmbientColors": white}));
        let cell = raw(json!({"AmbientColors": black}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        // The template only applies when the cell inherits something at all.
        assert_eq!(merged.lighting.ambient_cube.0[0], Vec3::ZERO);

        let inheriting = raw(json!({"AmbientColors": black, "Inherits": Inherits::FOG_MAX}));
        let merged = resolve(&inheriting, TemplateSource::Available(&template)).unwrap();
        assert_eq!(merged.lighting.ambient_cube.0[0], Vec3::ONE);
    }

    #[test]
    fn a_non_black_cell_ambient_cube_wins_over_the_template() {
        let white = cube(std::array::from_fn(|_| color(255, 255, 255)));
        let template = raw(json!({"AmbientColors": white}));
        let cell = raw(json!({"Inherits": Inherits::FOG_MAX}));
        let merged = resolve(&cell, TemplateSource::Available(&template)).unwrap();
        assert!(merged.lighting.ambient_cube.0[0].x < 0.2);
    }

    fn fog_of(lighting: &CellLighting) -> FogParams {
        match lighting.fog {
            Fog::Enabled(fog) => fog,
            Fog::Disabled => panic!("fog should be enabled"),
        }
    }

    #[test]
    fn cell_fog_takes_the_interior_colour_factors_and_no_height_or_sunlight() {
        let fog = fog_of(&resolved(json!({})).unwrap().lighting);
        let near = INTERIOR_COLOUR_FACTORS.near;
        let far = INTERIOR_COLOUR_FACTORS.far;
        assert_eq!(fog.colors.near, Vec3::new(0.0, 0.0, near));
        assert_eq!(fog.colors.far, Vec3::new(far, 0.0, 0.0));
        assert_eq!(fog.height, HeightBands::NONE);
        assert_eq!(fog.sunlight, SunlightTerm::NONE);
    }

    #[test]
    fn cell_fog_colours_decode_with_gamma_two_point_two() {
        let fog = fog_of(
            &resolved(json!({"FogFarColor": color(128, 128, 128)}))
                .unwrap()
                .lighting,
        );
        let expected = (128.0_f32 / 255.0).powf(2.2) * INTERIOR_COLOUR_FACTORS.far;
        assert!((fog.colors.far.x - expected).abs() < 1e-6);
    }

    #[test]
    fn cell_height_fog_fields_are_ignored() {
        let with_height = json!({
            "NearHeightMid": 500.0, "NearHeightRange": 1000.0,
            "FarHeightMid": 1500.0, "FarHeightRange": 3000.0,
            "FogHighNearColor": color(0, 255, 0),
            "HighDensityScale": 0.7,
        });
        assert_eq!(
            resolved(with_height).unwrap().lighting.fog,
            resolved(json!({})).unwrap().lighting.fog
        );
    }

    #[test]
    fn each_fog_colour_scale_multiplies_its_colour() {
        let fog = fog_of(
            &resolved(json!({"FogNearScale": 2.0, "FogFarScale": 3.0}))
                .unwrap()
                .lighting,
        );
        let plain = fog_of(&resolved(json!({})).unwrap().lighting);
        assert_eq!(fog.colors.near, plain.colors.near * 2.0);
        assert_eq!(fog.colors.far, plain.colors.far * 3.0);
    }

    #[test]
    fn a_record_that_stores_zero_for_every_scale_predates_them_and_is_unscaled() {
        let truncated = json!({"FogNearScale": 0.0, "FogFarScale": 0.0});
        assert_eq!(
            resolved(truncated).unwrap().lighting.fog,
            resolved(json!({})).unwrap().lighting.fog
        );
    }

    #[test]
    fn one_zero_scale_among_set_ones_is_taken_as_it_is() {
        let fog = fog_of(
            &resolved(json!({"FogNearScale": 1.0, "FogFarScale": 0.0}))
                .unwrap()
                .lighting,
        );
        assert_eq!(fog.colors.far, Vec3::ZERO);
        assert_ne!(fog.colors.near, Vec3::ZERO);
    }

    #[test]
    fn a_negative_fog_scale_is_rejected_naming_the_field() {
        for field in ["FogNearScale", "FogFarScale"] {
            let mut fields = json!({});
            fields[field] = json!(-1.0);
            assert!(
                matches!(resolved(fields), Err(CellLightingError::NegativeFogScale { field: named }) if named == field),
                "{field}"
            );
        }
    }
}
