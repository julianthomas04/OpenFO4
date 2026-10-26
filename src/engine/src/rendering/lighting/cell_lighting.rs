use glam::Vec3;
use serde::Deserialize;

use super::ambient::{AMBIENT_CUBE_FACES, AmbientCube};
use super::fog::{Fog, FogParams};
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
}

impl CellLightingError {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Malformed(_) => "malformed",
            Self::MissingField { .. } => "missing field",
            Self::NonFinite { .. } => "non-finite field",
            Self::InvertedFogRange { .. } => "inverted fog range",
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
        srgb8_to_linear([self.r, self.g, self.b])
    }
}

pub(crate) fn srgb_to_linear(channel: u8) -> f32 {
    let encoded = f32::from(channel) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
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
/// does not use (light fade distances, height fog, specular, scales) are ignored.
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
        let source = match self.template {
            Some(template) if self.inherits.contains(bit) => template,
            _ => self.cell,
        };
        get(source).ok_or(CellLightingError::MissingField { field })
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
        Ok(Fog::Enabled(FogParams {
            near_color: self
                .pick(Inherits::FOG_COLOR, "FogNearColor", |raw| {
                    raw.fog_near_color
                })?
                .linear(),
            far_color: self
                .pick(Inherits::FOG_COLOR, "FogFarColor", |raw| raw.fog_far_color)?
                .linear(),
            near: near * SCALE,
            far: far * SCALE,
            power: self.pick_finite(Inherits::FOG_POWER, "FogPower", |raw| raw.fog_power)?,
            max: self.pick_finite(Inherits::FOG_MAX, "FogMax", |raw| raw.fog_max)?,
        }))
    }

    fn clip_distance(&self) -> Result<Option<f32>, CellLightingError> {
        let distance = self.pick_finite(Inherits::CLIP_DISTANCE, "FogClipDistance", |raw| {
            raw.fog_clip_distance
        })?;
        Ok((distance > 0.0).then_some(distance * SCALE))
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
        assert_eq!(fog.near_color, Vec3::new(0.0, 0.0, 1.0));
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
}
