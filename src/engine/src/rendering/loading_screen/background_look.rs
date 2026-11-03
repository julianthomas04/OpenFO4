//! How the large loading screen's background is drawn. Every value is measured from a vanilla
//! screenshot against the game's own data, not documented by the game, and meant to be tuned.
//! The shader applies the same arithmetic as `graded` and `background_uv` to gamma-encoded
//! texels, because the menu draws through the non-sRGB view of the swapchain.

/// The right end of the background image's u range. The 2048 x 1024 texture holds a 16:9 image
/// 1820 px wide, padded to 2048; its right strip is unrelated scraps. The image is stretched, not
/// cropped, over any aspect: the background is one full-screen strip whose u runs from 0 to
/// this value, whatever the screen's shape.
pub const BACKGROUND_U_MAX: f32 = 16.0 / 9.0 / 2.0;

/// The factor of the additive grid overlay's alpha, which is blended with (source alpha, one).
pub const OVERLAY_ALPHA_FACTOR: f32 = 0.6;

/// The per-channel multiplier of the summed layers, in encoded space.
pub const GRADE_TINT: [f32; 3] = [0.662, 0.673, 0.489];

/// The per-channel offset added after the multiplier, in encoded space.
pub const GRADE_LIFT: [f32; 3] = [6.3 / 255.0, 5.5 / 255.0, 5.1 / 255.0];

/// The colour of the light on the 3D model, linear, scaled so that green is 1: the grade's
/// per-channel multiplier read as a gamma-encoded colour. Vanilla's model has the same warm cast
/// as its graded background (blue about 0.73 of green when encoded), so the one tint is taken to
/// colour both.
pub fn light_colour() -> [f32; 3] {
    use crate::rendering::lighting::cell_lighting::srgb_unit_to_linear;
    let linear = GRADE_TINT.map(srgb_unit_to_linear);
    linear.map(|channel| channel / linear[1])
}

/// The texture coordinates both layers are sampled at for a point of the screen: `screen_uv` runs
/// from (0, 0) at the top left to (1, 1) at the bottom right. The reference the shader's vertex
/// stage is tested against.
#[cfg(test)]
pub fn background_uv(screen_uv: [f32; 2]) -> [f32; 2] {
    [screen_uv[0] * BACKGROUND_U_MAX, screen_uv[1]]
}

/// The colour of a pixel from the encoded `background` texel and the encoded, non-premultiplied
/// `overlay` texel, before the fade. The reference the shader's fragment stage is tested against.
#[cfg(test)]
pub fn graded(background: [f32; 3], overlay: [f32; 4]) -> [f32; 3] {
    let overlay_gain = OVERLAY_ALPHA_FACTOR * overlay[3];
    std::array::from_fn(|channel| {
        GRADE_TINT[channel] * (background[channel] + overlay_gain * overlay[channel])
            + GRADE_LIFT[channel]
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the corners map to exactly representable values"
)]
mod tests {
    use super::*;

    #[test]
    fn the_background_is_the_images_sixteen_by_nine_part() {
        assert!((BACKGROUND_U_MAX - 0.889).abs() < 0.001);
        assert!((BACKGROUND_U_MAX * 2048.0 - 1820.0).abs() < 1.0);
    }

    #[test]
    fn the_screen_is_stretched_over_the_images_part_whatever_its_aspect() {
        assert_eq!(background_uv([0.0, 0.0]), [0.0, 0.0]);
        assert_eq!(background_uv([1.0, 1.0]), [BACKGROUND_U_MAX, 1.0]);
        assert_eq!(background_uv([0.5, 0.25]), [BACKGROUND_U_MAX / 2.0, 0.25]);
    }

    fn assert_near(actual: [f32; 3], expected: [f32; 3]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < 1e-5, "{actual:?} is not {expected:?}");
        }
    }

    #[test]
    fn without_an_overlay_the_background_is_tinted_and_lifted() {
        assert_near(
            graded([1.0, 0.5, 0.0], [1.0, 1.0, 1.0, 0.0]),
            [
                GRADE_TINT[0] + GRADE_LIFT[0],
                GRADE_TINT[1] * 0.5 + GRADE_LIFT[1],
                GRADE_LIFT[2],
            ],
        );
    }

    #[test]
    fn the_light_is_the_grade_tint_as_linear_light_scaled_to_green() {
        let [red, green, blue] = light_colour();
        assert_eq!(green, 1.0);
        assert!((red - 0.964).abs() < 0.002, "{red}");
        assert!((blue - 0.497).abs() < 0.002, "{blue}");
    }

    #[test]
    fn the_overlay_adds_its_colour_by_six_tenths_of_its_alpha() {
        let plain = graded([0.2; 3], [0.0; 4]);
        let lit = graded([0.2; 3], [0.5, 0.5, 0.5, 0.5]);
        for channel in 0..3 {
            let added = 0.6 * 0.5 * 0.5 * GRADE_TINT[channel];
            assert!((lit[channel] - plain[channel] - added).abs() < 1e-6);
        }
    }

    #[test]
    fn a_transparent_overlay_texel_adds_nothing_whatever_its_colour() {
        assert_eq!(
            graded([0.3; 3], [1.0, 1.0, 1.0, 0.0]),
            graded([0.3; 3], [0.0; 4])
        );
    }
}
