use super::fade::FadeRange;

/// How the lightning of a weather behaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightningParams {
    /// How far into a transition the thunder fades in and out; lightning follows it.
    pub fade: FadeRange,
    /// The record's frequency byte as a fraction of 255. The game inverts it: 1.0 (byte 255) is
    /// the rarest lightning and a byte of 15 the most frequent.
    pub frequency: f32,
    /// The flash colour, sRGB as authored. Rendering converts it.
    pub color_srgb: [u8; 3],
}

impl LightningParams {
    /// The record's frequency byte. A fraction outside `0..=1` saturates at the ends of the byte
    /// and NaN is byte 0.
    pub fn frequency_byte(&self) -> u8 {
        (self.frequency * 255.0).round() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_frequency(frequency: f32) -> LightningParams {
        LightningParams {
            fade: FadeRange {
                begin_fade_in: 0.0,
                end_fade_out: 1.0,
            },
            frequency,
            color_srgb: [255, 255, 255],
        }
    }

    #[test]
    fn every_frequency_byte_survives_the_fraction() {
        for byte in 0..=u8::MAX {
            let params = with_frequency(f32::from(byte) / 255.0);
            assert_eq!(params.frequency_byte(), byte, "{byte}");
        }
    }

    #[test]
    fn a_fraction_outside_the_byte_saturates_and_nan_is_zero() {
        assert_eq!(with_frequency(-0.5).frequency_byte(), 0);
        assert_eq!(with_frequency(1.5).frequency_byte(), 255);
        assert_eq!(with_frequency(f32::NAN).frequency_byte(), 0);
    }
}
