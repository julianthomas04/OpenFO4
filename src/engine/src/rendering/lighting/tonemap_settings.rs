use std::sync::Arc;

use glam::{Vec3, Vec4};

use super::lerp::{Lerp, lerp_fields};
use crate::assets::layout::named_texture_key;

/// Discriminants are mirrored by the `TONEMAP_*` constants in `shaders/tonemap.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum TonemapOperator {
    Filmic = 0,
    AgX = 1,
}

impl TonemapOperator {
    pub const ALL: [TonemapOperator; 2] = [TonemapOperator::Filmic, TonemapOperator::AgX];

    pub fn label(self) -> &'static str {
        match self {
            TonemapOperator::Filmic => "Filmic (measured vanilla)",
            TonemapOperator::AgX => "AgX",
        }
    }
}

/// The cinematic block of an image space and its tint. They act on the tone-mapped picture in
/// linear light, before the display encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSpaceCinematic {
    pub saturation: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub tint_amount: f32,
    /// Linear.
    pub tint_color: Vec3,
}

lerp_fields!(ImageSpaceCinematic {
    saturation,
    brightness,
    contrast,
    tint_amount,
    tint_color,
});

impl ImageSpaceCinematic {
    /// Leaves the picture as the tone curve made it.
    pub const NEUTRAL: Self = Self {
        saturation: 1.0,
        brightness: 1.0,
        contrast: 1.0,
        tint_amount: 0.0,
        tint_color: Vec3::ONE,
    };
}

impl Default for ImageSpaceCinematic {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// The colour lookup tables an image space grades the display picture with, each by its
/// [`named_texture_key`] with the weight it has; what the weights leave is the picture without a
/// table. One image space names at most one table; blending image spaces weighs their tables as
/// every other field is weighed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageSpaceLut {
    tables: Vec<(Arc<str>, f32)>,
}

impl ImageSpaceLut {
    /// The picture as the cinematic stage leaves it.
    pub const NONE: Self = Self { tables: Vec::new() };

    /// The table an image space names by its path below the textures folder; a blank path is none.
    pub fn named(path: &str) -> Self {
        Self::weighted(path, 1.0)
    }

    fn weighted(path: &str, weight: f32) -> Self {
        let mut lut = Self::NONE;
        if !path.trim().is_empty() {
            lut.add(named_texture_key(path.trim()).into(), weight);
        }
        lut
    }

    /// The tables with their weights, each texture key once and no weight zero.
    pub fn tables(&self) -> &[(Arc<str>, f32)] {
        &self.tables
    }

    /// The weight of all tables together: 0 shows the picture ungraded, 1 graded by the tables
    /// alone.
    pub fn amount(&self) -> f32 {
        self.tables.iter().map(|(_, weight)| weight).sum()
    }

    fn add(&mut self, key: Arc<str>, weight: f32) {
        if weight <= 0.0 {
            return;
        }
        match self.tables.iter_mut().find(|(known, _)| *known == key) {
            Some((_, total)) => *total += weight,
            None => self.tables.push((key, weight)),
        }
    }
}

/// Weighs the tables of both ends, so a graded picture fades into another table's or into the
/// ungraded one continuously. Unmeasured: no capture changed the table across a blend.
impl Lerp for ImageSpaceLut {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let mut blend = Self::NONE;
        for (key, weight) in &self.tables {
            blend.add(Arc::clone(key), weight * (1.0 - t));
        }
        for (key, weight) in &other.tables {
            blend.add(Arc::clone(key), weight * t);
        }
        blend
    }
}

#[derive(Debug, Clone)]
pub struct TonemapSettings {
    pub operator: TonemapOperator,
    pub cinematic: ImageSpaceCinematic,
    pub lut: ImageSpaceLut,
}

impl Default for TonemapSettings {
    fn default() -> Self {
        Self {
            operator: TonemapOperator::Filmic,
            cinematic: ImageSpaceCinematic::NEUTRAL,
            lut: ImageSpaceLut::NONE,
        }
    }
}

/// The frame uniform's tonemap fields, laid out as `frame.wgsl` documents them.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TonemapUniform {
    pub tonemap: Vec4,
    pub cinematic: Vec4,
    pub tint: Vec4,
}

impl TonemapSettings {
    pub fn uniform(&self, tonemap_e: f32) -> TonemapUniform {
        let cinematic = &self.cinematic;
        TonemapUniform {
            tonemap: Vec4::new(
                self.operator as u32 as f32,
                tonemap_e,
                self.lut.amount(),
                0.0,
            ),
            cinematic: Vec4::new(
                cinematic.saturation,
                cinematic.brightness,
                cinematic.contrast,
                cinematic.tint_amount,
            ),
            tint: cinematic.tint_color.extend(0.0),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the weights below are exactly representable fractions"
)]
mod tests {
    use super::*;

    const DAY: &str = r"Effects\LUTs\LUT_CW_ClearDAY.DDS";
    const NIGHT: &str = r"Effects\LUTs\LUT_CW_ClearNIGHT.DDS";

    fn weights(lut: &ImageSpaceLut) -> Vec<(&str, f32)> {
        lut.tables()
            .iter()
            .map(|(key, weight)| (&**key, *weight))
            .collect()
    }

    #[test]
    fn the_uniform_carries_the_operator_the_toe_the_lut_amount_and_the_cinematic_fields() {
        let uniform = TonemapSettings {
            operator: TonemapOperator::AgX,
            cinematic: ImageSpaceCinematic {
                saturation: 0.5,
                brightness: 2.0,
                contrast: 1.5,
                tint_amount: 0.25,
                tint_color: Vec3::new(1.0, 0.5, 0.0),
            },
            lut: ImageSpaceLut::NONE.lerp(&ImageSpaceLut::named(DAY), 0.75),
        }
        .uniform(0.04);
        assert_eq!(
            uniform,
            TonemapUniform {
                tonemap: Vec4::new(1.0, 0.04, 0.75, 0.0),
                cinematic: Vec4::new(0.5, 2.0, 1.5, 0.25),
                tint: Vec4::new(1.0, 0.5, 0.0, 0.0),
            }
        );
    }

    #[test]
    fn an_image_space_names_its_table_by_the_installed_texture_key_and_a_blank_path_names_none() {
        assert_eq!(
            weights(&ImageSpaceLut::named(DAY)),
            [(r"textures\effects\luts\lut_cw_clearday.dds", 1.0)]
        );
        assert_eq!(ImageSpaceLut::named(" "), ImageSpaceLut::NONE);
        assert_eq!(ImageSpaceLut::NONE.amount(), 0.0);
    }

    #[test]
    fn a_blend_weighs_the_tables_of_both_ends_and_the_rest_stays_ungraded() {
        let day = ImageSpaceLut::named(DAY);
        let night = ImageSpaceLut::named(NIGHT);
        let blend = day.lerp(&night, 0.25);
        assert_eq!(
            weights(&blend),
            [
                (r"textures\effects\luts\lut_cw_clearday.dds", 0.75),
                (r"textures\effects\luts\lut_cw_clearnight.dds", 0.25),
            ]
        );
        let fading = day.lerp(&ImageSpaceLut::NONE, 0.25);
        assert_eq!(fading.amount(), 0.75);
        assert_eq!(day.lerp(&night, 0.0), day);
        assert_eq!(day.lerp(&night, 1.0), night);
    }

    #[test]
    fn a_table_both_ends_name_keeps_its_whole_weight_however_they_spell_it() {
        let shouting = ImageSpaceLut::named(&DAY.to_ascii_uppercase());
        let blend = ImageSpaceLut::named(DAY).lerp(&shouting, 0.25);
        assert_eq!(blend, ImageSpaceLut::named(DAY));
    }

    #[test]
    fn blends_of_blends_keep_every_table_once() {
        let day = ImageSpaceLut::named(DAY);
        let night = ImageSpaceLut::named(NIGHT);
        let blend = day.lerp(&night, 0.5).lerp(&night.lerp(&day, 0.5), 0.5);
        assert_eq!(blend.tables().len(), 2);
        assert!((blend.amount() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn operator_discriminants_match_the_shader_constants() {
        assert_eq!(TonemapOperator::Filmic as u32, 0);
        assert_eq!(TonemapOperator::AgX as u32, 1);
        let shader = include_str!("../shaders/tonemap.wgsl");
        assert!(shader.contains("const TONEMAP_FILMIC = 0u;"));
        assert!(shader.contains("const TONEMAP_AGX = 1u;"));
    }
}
