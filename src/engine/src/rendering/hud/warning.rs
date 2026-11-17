//! The colour the HUD draws a warning in: the rollover text and the crosshair of an action the
//! game flags as a theft.

use crate::rendering::lighting::cell_lighting::srgb8_to_linear;

/// The 8-bit sRGB colour of a warning. Measured in the original, 2026-11-12, 18 reads, 3
/// backgrounds (no spread); not read from game data: no converted layout carries it.
pub const WARNING_RGB: [u8; 3] = [255, 94, 60];

/// [`WARNING_RGB`] as the linear light a HUD vertex carries.
pub fn warning_linear() -> [f32; 3] {
    srgb8_to_linear(WARNING_RGB).to_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linear_warning_decodes_each_srgb_channel() {
        let [r, g, b] = warning_linear();
        assert!((r - 1.0).abs() < 1e-6, "{r}");
        assert!((g - 0.1119).abs() < 1e-3, "{g}");
        assert!((b - 0.0452).abs() < 1e-3, "{b}");
    }
}
