//! Reimplementation of the perk colour adjustment using behavior observed at runtime and
//! constants extracted from the installed game's movie. This code does not copy the movie's
//! `ActionScript` implementation.

use super::layout::MOVIE;
use crate::rendering::pipboy::art::ColourMatrix;
use crate::ui_layout::{KeyGroup, LiteralKey, NumberKey, UiLayout, UiLayoutError};

macro_rules! literal {
    ($name:ident, $key:literal) => {
        pub(super) static $name: LiteralKey = LiteralKey::new(MOVIE, $key, 1);
    };
}

literal!(BRIGHTNESS, "PerkGrid.<init>#$1.brightness=");
literal!(CONTRAST, "PerkGrid.<init>#$1.contrast=");
literal!(SATURATION, "PerkGrid.<init>#$1.saturation=");
literal!(HUE, "PerkGrid.<init>#$1.hue=");
literal!(CONTRAST_PERCENT, "fl.motion.AdjustColor.set:contrast#$1/");
literal!(
    CONTRAST_NEGATIVE_RANGE,
    "fl.motion.AdjustColor.set:contrast#($1/100)*"
);
literal!(
    CONTRAST_NEGATIVE_MIDDLE,
    "fl.motion.AdjustColor.set:contrast#(($1/100)*127)+"
);
literal!(
    CONTRAST_MATRIX_RANGE,
    "fl.motion.ColorMatrix.SetContrastMatrix#$1/"
);
literal!(
    CONTRAST_MATRIX_HALF,
    "fl.motion.ColorMatrix.SetContrastMatrix#*(127-$1)"
);
literal!(
    CONTRAST_MATRIX_MIDDLE,
    "fl.motion.ColorMatrix.SetContrastMatrix#-$1"
);
literal!(
    SATURATION_PERCENT,
    "fl.motion.AdjustColor.set:saturation#$1/"
);
literal!(
    SATURATION_NEGATIVE_BASE,
    "fl.motion.AdjustColor.set:saturation#($1/100)+"
);
literal!(
    SATURATION_MATRIX_ONE,
    "fl.motion.ColorMatrix.SetSaturationMatrix#-$1"
);
pub(super) static LUMINANCE_RED: NumberKey =
    NumberKey::new(MOVIE, "fl.motion.ColorMatrix.LUMINANCER");
pub(super) static LUMINANCE_GREEN: NumberKey =
    NumberKey::new(MOVIE, "fl.motion.ColorMatrix.LUMINANCEG");
pub(super) static LUMINANCE_BLUE: NumberKey =
    NumberKey::new(MOVIE, "fl.motion.ColorMatrix.LUMINANCEB");

pub static LAYOUT_KEYS: KeyGroup = KeyGroup(&[
    &BRIGHTNESS,
    &CONTRAST,
    &SATURATION,
    &HUE,
    &CONTRAST_PERCENT,
    &CONTRAST_NEGATIVE_RANGE,
    &CONTRAST_NEGATIVE_MIDDLE,
    &CONTRAST_MATRIX_RANGE,
    &CONTRAST_MATRIX_HALF,
    &CONTRAST_MATRIX_MIDDLE,
    &SATURATION_PERCENT,
    &SATURATION_NEGATIVE_BASE,
    &SATURATION_MATRIX_ONE,
    &LUMINANCE_RED,
    &LUMINANCE_GREEN,
    &LUMINANCE_BLUE,
]);

/// A colour matrix filter holds its offsets in 0..255 channel units.
const OFFSET_UNITS: f32 = 255.0;

/// The constants of the adjustment arithmetic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdjustRules {
    pub contrast_percent: f32,
    pub contrast_negative_range: f32,
    pub contrast_negative_middle: f32,
    pub contrast_matrix_range: f32,
    pub contrast_matrix_half: f32,
    pub contrast_matrix_middle: f32,
    pub saturation_percent: f32,
    pub saturation_negative_base: f32,
    pub saturation_matrix_one: f32,
    pub luminance: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjustment {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub hue: f32,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AdjustError {
    #[error("the colour adjustment has a hue of {0}; only an unchanged hue is supported")]
    Hue(f32),
    #[error("the colour adjustment has a contrast of {0}; only negative contrast is supported")]
    Contrast(f32),
    #[error("the colour adjustment has a saturation of {0}; only negative saturation is supported")]
    Saturation(f32),
}

type Matrix5 = [[f32; 5]; 5];

fn identity() -> Matrix5 {
    let mut matrix = [[0.0; 5]; 5];
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    matrix
}

/// `left` applied after `right`.
fn compose(left: &Matrix5, right: &Matrix5) -> Matrix5 {
    let mut product = [[0.0; 5]; 5];
    for (row, product_row) in product.iter_mut().enumerate() {
        for (column, cell) in product_row.iter_mut().enumerate() {
            *cell = (0..5).map(|k| left[row][k] * right[k][column]).sum();
        }
    }
    product
}

/// The matrix `brightness`, then `contrast`, then `saturation` build (hue unchanged).
pub fn adjust_colour(
    rules: &AdjustRules,
    adjustment: Adjustment,
) -> Result<ColourMatrix, AdjustError> {
    if adjustment.hue != 0.0 {
        return Err(AdjustError::Hue(adjustment.hue));
    }
    if adjustment.contrast >= 0.0 {
        return Err(AdjustError::Contrast(adjustment.contrast));
    }
    if adjustment.saturation >= 0.0 {
        return Err(AdjustError::Saturation(adjustment.saturation));
    }
    let mut brightness = identity();
    if adjustment.brightness != 0.0 {
        for row in brightness.iter_mut().take(3) {
            row[4] = adjustment.brightness;
        }
    }

    let contrast_value = adjustment.contrast / rules.contrast_percent
        * rules.contrast_negative_range
        + rules.contrast_negative_middle;
    let mut contrast = identity();
    let contrast_offset =
        rules.contrast_matrix_half * (rules.contrast_matrix_middle - contrast_value);
    let contrast_scale = contrast_value / rules.contrast_matrix_range;
    for (channel, row) in contrast.iter_mut().enumerate().take(3) {
        row[channel] = contrast_scale;
        row[4] = contrast_offset;
    }

    let saturation_value =
        adjustment.saturation / rules.saturation_percent + rules.saturation_negative_base;
    let remainder = rules.saturation_matrix_one - saturation_value;
    let mut saturation = identity();
    for (index, row) in saturation.iter_mut().enumerate().take(3) {
        for (column, weight) in rules.luminance.iter().enumerate() {
            row[column] = remainder * weight;
        }
        row[index] += saturation_value;
    }

    let combined = compose(&saturation, &compose(&contrast, &brightness));
    let mut rows = [[0.0; 5]; 4];
    for (row, target) in rows.iter_mut().enumerate() {
        for (column, cell) in target.iter_mut().enumerate() {
            *cell = if column == 4 {
                combined[row][column] / OFFSET_UNITS
            } else {
                combined[row][column]
            };
        }
    }
    Ok(ColourMatrix { rows })
}

fn single(layout: &UiLayout, key: &LiteralKey) -> Result<f32, UiLayoutError> {
    Ok(layout.lit_number(key, 0)? as f32)
}

/// The filter the grid puts on perks that can be taken but are not held.
pub fn unavailable_filter(layout: &UiLayout) -> Result<ColourMatrix, UiLayoutError> {
    let rules = AdjustRules {
        contrast_percent: single(layout, &CONTRAST_PERCENT)?,
        contrast_negative_range: single(layout, &CONTRAST_NEGATIVE_RANGE)?,
        contrast_negative_middle: single(layout, &CONTRAST_NEGATIVE_MIDDLE)?,
        contrast_matrix_range: single(layout, &CONTRAST_MATRIX_RANGE)?,
        contrast_matrix_half: single(layout, &CONTRAST_MATRIX_HALF)?,
        contrast_matrix_middle: single(layout, &CONTRAST_MATRIX_MIDDLE)?,
        saturation_percent: single(layout, &SATURATION_PERCENT)?,
        saturation_negative_base: single(layout, &SATURATION_NEGATIVE_BASE)?,
        saturation_matrix_one: single(layout, &SATURATION_MATRIX_ONE)?,
        luminance: [
            layout.get(&LUMINANCE_RED)? as f32,
            layout.get(&LUMINANCE_GREEN)? as f32,
            layout.get(&LUMINANCE_BLUE)? as f32,
        ],
    };
    let adjustment = Adjustment {
        brightness: single(layout, &BRIGHTNESS)?,
        contrast: single(layout, &CONTRAST)?,
        saturation: single(layout, &SATURATION)?,
        hue: single(layout, &HUE)?,
    };
    adjust_colour(&rules, adjustment)
        .map_err(|error| UiLayoutError::unresolved(MOVIE, CONTRAST.path, error.to_string()))
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;

    const RULES: AdjustRules = AdjustRules {
        contrast_percent: 100.0,
        contrast_negative_range: 127.0,
        contrast_negative_middle: 127.0,
        contrast_matrix_range: 127.0,
        contrast_matrix_half: 0.5,
        contrast_matrix_middle: 127.0,
        saturation_percent: 100.0,
        saturation_negative_base: 1.0,
        saturation_matrix_one: 1.0,
        luminance: [0.3086, 0.6094, 0.082],
    };

    const ADJUSTMENT: Adjustment = Adjustment {
        brightness: 67.0,
        contrast: -48.0,
        saturation: -51.0,
        hue: 0.0,
    };

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn the_adjustment_composes_brightness_contrast_and_saturation_in_that_order() {
        let matrix = adjust_colour(&RULES, ADJUSTMENT).unwrap();

        close(matrix.rows[0][0], 0.3366);
        close(matrix.rows[0][1], 0.1616);
        close(matrix.rows[0][2], 0.0217);
        close(matrix.rows[1][0], 0.0818);
        close(matrix.rows[1][1], 0.4164);
        close(matrix.rows[2][2], 0.2765);
        for channel in 0..3 {
            close(matrix.rows[channel][4] * OFFSET_UNITS, 65.32);
        }
    }

    #[test]
    fn alpha_passes_through_unchanged() {
        let matrix = adjust_colour(&RULES, ADJUSTMENT).unwrap();
        assert_eq!(matrix.rows[3], [0.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn unsupported_branches_of_the_arithmetic_are_named_errors() {
        let with = |change: fn(&mut Adjustment)| {
            let mut adjustment = ADJUSTMENT;
            change(&mut adjustment);
            adjust_colour(&RULES, adjustment).unwrap_err()
        };
        assert_eq!(with(|a| a.hue = 30.0), AdjustError::Hue(30.0));
        assert_eq!(with(|a| a.contrast = 10.0), AdjustError::Contrast(10.0));
        assert_eq!(with(|a| a.saturation = 0.0), AdjustError::Saturation(0.0));
    }
}
