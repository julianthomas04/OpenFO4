//! The colour lookup tables image spaces grade the display picture with. The game stores each as
//! an uncompressed 24 bit DDS strip of `size` slices of `size` × `size` texels side by side: the
//! texel at column `r + size · b`, row `g` holds the graded colour of the encoded display colour
//! `(r, g, b) / (size - 1)`. Every vanilla table is 16 texels a side.

use std::io;

use glam::Vec3;
use thiserror::Error;

/// Texels per side of every table the tonemap pass grades with.
pub const LUT_SIZE: u32 = 16;

const LUT_TEXELS: usize = (LUT_SIZE * LUT_SIZE * LUT_SIZE) as usize;
const STRIP_BIT_COUNT: u32 = 24;
const STRIP_MASKS: [u32; 3] = [0x00ff_0000, 0x0000_ff00, 0x0000_00ff];
const BYTES_PER_STRIP_TEXEL: usize = 3;

#[derive(Debug, Error)]
pub enum LutTableError {
    #[error("not a DDS file: {0}")]
    Dds(#[from] ddsfile::Error),
    #[error(
        "the texels are not 24 bit RGB (bit count {bit_count:?}, masks {masks:x?}); a colour LUT is"
    )]
    Format {
        bit_count: Option<u32>,
        masks: [Option<u32>; 3],
    },
    #[error(
        "the strip is {width} × {height} texels; a colour LUT is {} × {LUT_SIZE}",
        LUT_SIZE * LUT_SIZE
    )]
    Size { width: u32, height: u32 },
    #[error("the strip holds {actual} bytes of texels, {expected} are needed")]
    Truncated { expected: usize, actual: usize },
}

/// One table: the graded colour of each grid point, red fastest, then green, then blue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutTable {
    texels: Vec<[u8; 3]>,
}

impl LutTable {
    /// The table that leaves every colour as it is.
    pub fn identity() -> Self {
        Self::from_fn(|r, g, b| [r, g, b].map(grid_level))
    }

    /// The table that grades the grid point `(r, g, b)` to `graded(r, g, b)`.
    pub fn from_fn(graded: impl Fn(u32, u32, u32) -> [u8; 3]) -> Self {
        let texels = (0..LUT_TEXELS as u32)
            .map(|index| {
                graded(
                    index % LUT_SIZE,
                    index / LUT_SIZE % LUT_SIZE,
                    index / (LUT_SIZE * LUT_SIZE),
                )
            })
            .collect();
        Self { texels }
    }

    /// The first mip of a game strip; further mips, which some tables carry, are not read.
    pub fn from_dds(bytes: &[u8]) -> Result<Self, LutTableError> {
        let dds = ddsfile::Dds::read(io::Cursor::new(bytes))?;
        let format = &dds.header.spf;
        let masks = [format.r_bit_mask, format.g_bit_mask, format.b_bit_mask];
        if format.rgb_bit_count != Some(STRIP_BIT_COUNT) || masks != STRIP_MASKS.map(Some) {
            return Err(LutTableError::Format {
                bit_count: format.rgb_bit_count,
                masks,
            });
        }
        let (width, height) = (dds.header.width, dds.header.height);
        if width != LUT_SIZE * LUT_SIZE || height != LUT_SIZE {
            return Err(LutTableError::Size { width, height });
        }
        let expected = LUT_TEXELS * BYTES_PER_STRIP_TEXEL;
        let strip = dds.data.get(..expected).ok_or(LutTableError::Truncated {
            expected,
            actual: dds.data.len(),
        })?;
        let texel = |column: u32, row: u32| {
            let at = (row * width + column) as usize * BYTES_PER_STRIP_TEXEL;
            let [b, g, r] = [strip[at], strip[at + 1], strip[at + 2]];
            [r, g, b]
        };
        Ok(Self::from_fn(|r, g, b| texel(r + LUT_SIZE * b, g)))
    }

    #[cfg(test)]
    fn at(&self, r: u32, g: u32, b: u32) -> [u8; 3] {
        self.texels[((b * LUT_SIZE + g) * LUT_SIZE + r) as usize]
    }

    /// The graded colour of the encoded display colour `display` (0 to 1), interpolated
    /// trilinearly between the grid points around it, as the tonemap pass samples the table.
    #[cfg(test)]
    pub fn sample(&self, display: Vec3) -> Vec3 {
        let grid = display.clamp(Vec3::ZERO, Vec3::ONE) * (LUT_SIZE - 1) as f32;
        let low = grid.floor().min(Vec3::splat((LUT_SIZE - 2) as f32));
        let fraction = grid - low;
        let [r, g, b] = low.to_array().map(|x| x as u32);
        let mut graded = Vec3::ZERO;
        for corner in 0..8_u32 {
            let step = [corner & 1, corner >> 1 & 1, corner >> 2 & 1];
            let weight = (0..3)
                .map(|axis| {
                    if step[axis] == 1 {
                        fraction[axis]
                    } else {
                        1.0 - fraction[axis]
                    }
                })
                .product::<f32>();
            let texel = self.at(r + step[0], g + step[1], b + step[2]);
            graded += weight * Vec3::from(texel.map(f32::from)) / 255.0;
        }
        graded
    }
}

/// The level of grid point `index` in the encoded display picture.
fn grid_level(index: u32) -> u8 {
    (index * 255 / (LUT_SIZE - 1)) as u8
}

/// The RGBA texels of the one table the tonemap pass samples for `tables`, red along x, green
/// along y, blue along z: each grid point the weighted mean of the tables' graded colours. A table
/// that is `None` (not loadable) counts as the identity, so its weight leaves the picture as it is.
/// The tonemap pass mixes the sampled colour into the picture by the weights' sum.
pub fn blended_texels(tables: &[(Option<&LutTable>, f32)]) -> Vec<[u8; 4]> {
    let identity = LutTable::identity();
    let total: f32 = tables.iter().map(|(_, weight)| weight).sum();
    if total <= 0.0 {
        return identity
            .texels
            .iter()
            .map(|&[r, g, b]| [r, g, b, u8::MAX])
            .collect();
    }
    (0..LUT_TEXELS)
        .map(|index| {
            let graded = tables.iter().fold(Vec3::ZERO, |sum, (table, weight)| {
                let texel = table.unwrap_or(&identity).texels[index];
                sum + Vec3::from(texel.map(f32::from)) * (weight / total)
            });
            let [r, g, b] = graded.round().to_array().map(|x| x.clamp(0.0, 255.0) as u8);
            [r, g, b, u8::MAX]
        })
        .collect()
}

/// A game strip of `table`: 24 bit texels stored blue, green, red, as the game stores them,
/// followed by `extra` bytes of further mips.
#[cfg(test)]
pub(crate) fn strip_dds(table: &LutTable, extra: usize) -> Vec<u8> {
    let mut dds = ddsfile::Dds::new_d3d(ddsfile::NewD3dParams {
        height: LUT_SIZE,
        width: LUT_SIZE * LUT_SIZE,
        depth: None,
        format: ddsfile::D3DFormat::R8G8B8,
        mipmap_levels: None,
        caps2: None,
    })
    .unwrap();
    dds.data = (0..LUT_SIZE)
        .flat_map(|row| (0..LUT_SIZE * LUT_SIZE).map(move |column| (row, column)))
        .flat_map(|(g, column)| {
            let [r, g, b] = table.at(column % LUT_SIZE, g, column / LUT_SIZE);
            [b, g, r]
        })
        .chain(std::iter::repeat_n(0, extra))
        .collect();
    let mut bytes = Vec::new();
    dds.write(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every grid point graded to a different colour that tells all three coordinates apart.
    fn distinct() -> LutTable {
        LutTable::from_fn(|r, g, b| [(r * 16 + g) as u8, (g * 16 + b) as u8, (b * 16 + r) as u8])
    }

    #[test]
    fn a_strip_reads_back_as_the_table_it_stores_with_or_without_further_mips() {
        for extra in [0, 4125] {
            assert_eq!(
                LutTable::from_dds(&strip_dds(&distinct(), extra)).unwrap(),
                distinct()
            );
        }
    }

    #[test]
    fn the_strip_lays_the_blue_slices_side_by_side_and_green_down_each_slice() {
        let bytes = strip_dds(&distinct(), 0);
        let header = bytes.len() - LUT_TEXELS * BYTES_PER_STRIP_TEXEL;
        let (r, g, b) = (3_u32, 5_u32, 7_u32);
        let at = header + ((g * LUT_SIZE * LUT_SIZE + r + LUT_SIZE * b) as usize) * 3;
        assert_eq!(
            [bytes[at + 2], bytes[at + 1], bytes[at]],
            distinct().at(r, g, b)
        );
    }

    #[test]
    fn a_strip_of_another_format_size_or_length_is_rejected() {
        let mut rgba = ddsfile::Dds::new_d3d(ddsfile::NewD3dParams {
            height: LUT_SIZE,
            width: LUT_SIZE * LUT_SIZE,
            depth: None,
            format: ddsfile::D3DFormat::A8R8G8B8,
            mipmap_levels: None,
            caps2: None,
        })
        .unwrap();
        rgba.data = vec![0; LUT_TEXELS * 4];
        let mut bytes = Vec::new();
        rgba.write(&mut bytes).unwrap();
        assert!(matches!(
            LutTable::from_dds(&bytes),
            Err(LutTableError::Format { .. })
        ));

        let mut wide = strip_dds(&distinct(), 0);
        wide[16..20].copy_from_slice(&512_u32.to_le_bytes());
        assert!(matches!(
            LutTable::from_dds(&wide),
            Err(LutTableError::Size { width: 512, .. })
        ));

        let full = strip_dds(&distinct(), 0);
        assert!(matches!(
            LutTable::from_dds(&full[..full.len() - 1]),
            Err(LutTableError::Truncated { .. })
        ));
        assert!(matches!(
            LutTable::from_dds(b"not a dds"),
            Err(LutTableError::Dds(_))
        ));
    }

    #[test]
    fn the_identity_leaves_every_colour_as_it_is() {
        let identity = LutTable::identity();
        for display in [Vec3::ZERO, Vec3::ONE, Vec3::new(0.1, 0.5, 0.93)] {
            assert!(
                identity.sample(display).abs_diff_eq(display, 1e-6),
                "{display}"
            );
        }
    }

    #[test]
    fn sampling_hits_the_grid_points_and_interpolates_linearly_between_them() {
        let table = distinct();
        let grid_point = |r: u32, g: u32, b: u32| Vec3::new(r as f32, g as f32, b as f32) / 15.0;
        let texel = |r, g, b| Vec3::from(table.at(r, g, b).map(f32::from)) / 255.0;
        assert!(
            table
                .sample(grid_point(3, 5, 7))
                .abs_diff_eq(texel(3, 5, 7), 1e-6)
        );
        let between = (grid_point(3, 5, 7) + grid_point(4, 5, 7)) / 2.0;
        let expected = (texel(3, 5, 7) + texel(4, 5, 7)) / 2.0;
        assert!(table.sample(between).abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn a_single_table_blends_to_itself_and_an_unloadable_one_to_the_identity() {
        let as_rgba = |table: &LutTable| -> Vec<[u8; 4]> {
            table
                .texels
                .iter()
                .map(|&[r, g, b]| [r, g, b, 255])
                .collect()
        };
        assert_eq!(
            blended_texels(&[(Some(&distinct()), 0.3)]),
            as_rgba(&distinct())
        );
        assert_eq!(
            blended_texels(&[(None, 0.3)]),
            as_rgba(&LutTable::identity())
        );
        assert_eq!(blended_texels(&[]), as_rgba(&LutTable::identity()));
    }

    #[test]
    fn two_tables_blend_by_their_share_of_the_weights() {
        let black = LutTable::from_fn(|_, _, _| [0, 0, 0]);
        let white = LutTable::from_fn(|_, _, _| [200, 100, 40]);
        let blended = blended_texels(&[(Some(&black), 0.25), (Some(&white), 0.25)]);
        assert!(blended.iter().all(|&texel| texel == [100, 50, 20, 255]));
    }
}
