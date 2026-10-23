/// The luma coefficients that define how `Y'CbCr` relates to R'G'B'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourMatrix {
    Bt601,
    Bt709,
}

impl ColourMatrix {
    /// The red and blue luma weights of the standard.
    fn kr_kb(self) -> (f32, f32) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
        }
    }
}

/// How code values map onto the signal range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourRange {
    /// Luma 16 to 235, chroma 16 to 240.
    Limited,
    /// 0 to 255 for every component.
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColourError {
    #[error("the video declares no usable colour matrix ({0})")]
    UnknownMatrix(String),
    #[error("the video declares no usable colour range ({0})")]
    UnknownRange(String),
}

/// Where the half-resolution chroma samples sit relative to the luma samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaSiting {
    /// On a luma column (left-sited) rather than between two.
    pub horizontally_cosited: bool,
    /// On a luma row rather than between two.
    pub vertically_cosited: bool,
}

impl ChromaSiting {
    /// What H.264 and MPEG-2 use unless the stream says otherwise.
    pub const MPEG2: Self = Self {
        horizontally_cosited: true,
        vertically_cosited: false,
    };

    /// How far the chroma lookup moves, in luma pixels, so that bilinear sampling of the
    /// half-size planes (which assumes centre siting) lands on the true sample positions.
    pub fn sampling_shift(self) -> [f32; 2] {
        [
            if self.horizontally_cosited { 0.5 } else { 0.0 },
            if self.vertically_cosited { 0.5 } else { 0.0 },
        ]
    }
}

/// `rgb = rows * (ycbcr - bias)`, on components normalised to 0..1. The result is the gamma
/// encoded R'G'B' of the source, which the menu draws as is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YuvToRgb {
    pub bias: [f32; 3],
    pub rows: [[f32; 3]; 3],
}

impl YuvToRgb {
    pub fn new(matrix: ColourMatrix, range: ColourRange) -> Self {
        let (kr, kb) = matrix.kr_kb();
        let kg = 1.0 - kr - kb;
        let (luma_bias, luma_scale, chroma_scale) = match range {
            ColourRange::Limited => (16.0 / 255.0, 255.0 / 219.0, 255.0 / 224.0),
            ColourRange::Full => (0.0, 1.0, 1.0),
        };
        let red_from_cr = 2.0 * (1.0 - kr) * chroma_scale;
        let blue_from_cb = 2.0 * (1.0 - kb) * chroma_scale;
        let green_from_cb = -2.0 * kb * (1.0 - kb) / kg * chroma_scale;
        let green_from_cr = -2.0 * kr * (1.0 - kr) / kg * chroma_scale;
        Self {
            bias: [luma_bias, 128.0 / 255.0, 128.0 / 255.0],
            rows: [
                [luma_scale, 0.0, red_from_cr],
                [luma_scale, green_from_cb, green_from_cr],
                [luma_scale, blue_from_cb, 0.0],
            ],
        }
    }

    /// The conversion of one pixel given as 8-bit code values, unclamped.
    pub fn apply(&self, ycbcr: [u8; 3]) -> [f32; 3] {
        let centred: [f32; 3] = std::array::from_fn(|i| f32::from(ycbcr[i]) / 255.0 - self.bias[i]);
        self.rows.map(|row| {
            row.iter()
                .zip(centred)
                .map(|(weight, value)| weight * value)
                .sum()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb8(conversion: &YuvToRgb, ycbcr: [u8; 3]) -> [i32; 3] {
        conversion
            .apply(ycbcr)
            .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as i32)
    }

    fn close(actual: [i32; 3], expected: [i32; 3]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() <= 1, "{actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn limited_bt601_maps_black_white_and_grey() {
        let c = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        assert_eq!(rgb8(&c, [16, 128, 128]), [0, 0, 0]);
        assert_eq!(rgb8(&c, [235, 128, 128]), [255, 255, 255]);
        close(rgb8(&c, [126, 128, 128]), [128, 128, 128]);
    }

    #[test]
    fn limited_bt601_maps_the_primaries() {
        let c = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        close(rgb8(&c, [81, 90, 240]), [255, 0, 0]);
        close(rgb8(&c, [145, 54, 34]), [0, 255, 0]);
        close(rgb8(&c, [41, 240, 110]), [0, 0, 255]);
    }

    #[test]
    fn bt709_differs_from_bt601_for_the_same_code_values() {
        let c601 = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        let c709 = YuvToRgb::new(ColourMatrix::Bt709, ColourRange::Limited);
        close(rgb8(&c709, [63, 102, 240]), [255, 0, 0]);
        assert_ne!(rgb8(&c601, [63, 102, 240]), rgb8(&c709, [63, 102, 240]));
    }

    #[test]
    fn full_range_uses_the_whole_code_range() {
        let c = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Full);
        assert_eq!(rgb8(&c, [0, 128, 128]), [0, 0, 0]);
        assert_eq!(rgb8(&c, [255, 128, 128]), [255, 255, 255]);
        close(rgb8(&c, [76, 85, 255]), [255, 0, 0]);
    }

    #[test]
    fn limited_range_clips_below_black_and_above_white() {
        let c = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        assert_eq!(rgb8(&c, [0, 128, 128]), [0, 0, 0]);
        assert_eq!(rgb8(&c, [255, 128, 128]), [255, 255, 255]);
    }
}
