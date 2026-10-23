use std::time::Duration;

use super::colour::{ChromaSiting, YuvToRgb};
use super::error::VideoError;

/// One plane of 8-bit samples; rows are `stride` bytes apart.
#[derive(Debug, Clone, Copy)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
}

/// A decoded frame as three planes: full-size luma and half-size Cb and Cr (I420).
#[derive(Debug, Clone, Copy)]
pub struct PlanarFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub planes: [Plane<'a>; 3],
}

impl PlanarFrame<'_> {
    /// The size of plane `index` in samples.
    pub fn plane_size(&self, index: usize) -> (u32, u32) {
        plane_size((self.width, self.height), index)
    }
}

/// The size of plane `index` of an I420 frame of `frame` samples.
pub fn plane_size(frame: (u32, u32), index: usize) -> (u32, u32) {
    if index == 0 {
        frame
    } else {
        (frame.0.div_ceil(2), frame.1.div_ceil(2))
    }
}

/// When a frame is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// The position in the video file; it starts over at every loop.
    pub stream_time: Duration,
    /// The position on the playback timeline; it keeps counting across loops.
    pub running_time: Duration,
}

/// The visible part of a decoded picture, in luma samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// How the decoder lays an I420 picture out in its buffer: three planes back to back, the luma
/// plane `stride` bytes per row and `rows` rows tall (the coded height, which may exceed the
/// visible one), each chroma plane half as wide and half as tall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferLayout {
    pub stride: usize,
    pub rows: usize,
    pub visible: Area,
}

impl BufferLayout {
    fn chroma_stride(&self) -> usize {
        self.stride / 2
    }

    fn chroma_rows(&self) -> usize {
        self.rows.div_ceil(2)
    }

    fn required_len(&self) -> usize {
        self.stride * self.rows + 2 * self.chroma_stride() * self.chroma_rows()
    }

    /// Copies the visible picture out of `buffer` into tightly packed I420 planes.
    pub fn copy_visible(&self, buffer: &[u8]) -> Result<Vec<u8>, VideoError> {
        let Area {
            x,
            y,
            width,
            height,
        } = self.visible;
        if x % 2 != 0 || y % 2 != 0 {
            return Err(VideoError::UnsupportedFrame(format!(
                "the picture starts at the odd position {x}, {y}"
            )));
        }
        let (x, y) = (x as usize, y as usize);
        if x + width as usize > self.stride || y + height as usize > self.rows {
            return Err(VideoError::UnsupportedFrame(format!(
                "the visible area {:?} exceeds the {} x {} buffer",
                self.visible, self.stride, self.rows
            )));
        }
        if buffer.len() < self.required_len() {
            return Err(VideoError::UnsupportedFrame(format!(
                "the buffer holds {} bytes, the layout needs {}",
                buffer.len(),
                self.required_len()
            )));
        }
        let (luma, chroma) = buffer.split_at(self.stride * self.rows);
        let (cb, cr) = chroma.split_at(self.chroma_stride() * self.chroma_rows());
        let mut packed = Vec::with_capacity(i420_len((width, height)));
        let mut copy = |plane: &[u8], stride: usize, index: usize, (left, top): (usize, usize)| {
            let (columns, rows) = plane_size((width, height), index);
            for row in top..top + rows as usize {
                let start = row * stride + left;
                packed.extend_from_slice(&plane[start..start + columns as usize]);
            }
        };
        copy(luma, self.stride, 0, (x, y));
        copy(cb, self.chroma_stride(), 1, (x / 2, y / 2));
        copy(cr, self.chroma_stride(), 2, (x / 2, y / 2));
        Ok(packed)
    }
}

/// The bytes of a tightly packed I420 frame of `size` samples.
fn i420_len(size: (u32, u32)) -> usize {
    (0..3)
        .map(|index| {
            let (width, height) = plane_size(size, index);
            width as usize * height as usize
        })
        .sum()
}

/// A decoded frame in tightly packed I420 with a known colour conversion.
#[derive(Debug)]
pub struct DecodedFrame {
    size: (u32, u32),
    data: Vec<u8>,
    conversion: YuvToRgb,
    siting: ChromaSiting,
    timing: FrameTiming,
}

impl DecodedFrame {
    pub fn new(
        size: (u32, u32),
        data: Vec<u8>,
        conversion: YuvToRgb,
        siting: ChromaSiting,
        timing: FrameTiming,
    ) -> Result<Self, VideoError> {
        if data.len() != i420_len(size) {
            return Err(VideoError::UnsupportedFrame(format!(
                "{} bytes for a {} x {} I420 frame",
                data.len(),
                size.0,
                size.1
            )));
        }
        Ok(Self {
            size,
            data,
            conversion,
            siting,
            timing,
        })
    }

    /// A black 2 x 2 frame with the colorimetry of the converted video.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::new(
            (2, 2),
            vec![16, 16, 16, 16, 128, 128],
            YuvToRgb::new(
                super::colour::ColourMatrix::Bt601,
                super::colour::ColourRange::Limited,
            ),
            ChromaSiting::MPEG2,
            FrameTiming {
                stream_time: Duration::ZERO,
                running_time: Duration::ZERO,
            },
        )
        .expect("a valid frame")
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn conversion(&self) -> YuvToRgb {
        self.conversion
    }

    pub fn chroma_siting(&self) -> ChromaSiting {
        self.siting
    }

    pub fn timing(&self) -> FrameTiming {
        self.timing
    }

    pub fn planes(&self) -> PlanarFrame<'_> {
        let (luma, chroma) = self
            .data
            .split_at(i420_len(self.size) - 2 * chroma_len(self.size));
        let (cb, cr) = chroma.split_at(chroma_len(self.size));
        let stride = |index| plane_size(self.size, index).0 as usize;
        PlanarFrame {
            width: self.size.0,
            height: self.size.1,
            planes: [
                Plane {
                    data: luma,
                    stride: stride(0),
                },
                Plane {
                    data: cb,
                    stride: stride(1),
                },
                Plane {
                    data: cr,
                    stride: stride(2),
                },
            ],
        }
    }
}

fn chroma_len(size: (u32, u32)) -> usize {
    let (width, height) = plane_size(size, 1);
    width as usize * height as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{ColourMatrix, ColourRange};

    #[test]
    fn chroma_planes_are_half_size_rounded_up() {
        assert_eq!(plane_size((1920, 1080), 0), (1920, 1080));
        assert_eq!(plane_size((1920, 1080), 1), (960, 540));
        assert_eq!(plane_size((1920, 1080), 2), (960, 540));
        assert_eq!(plane_size((5, 3), 1), (3, 2));
    }

    /// A 4 x 4 buffer with stride 6 and 6 coded rows whose samples encode their plane and position.
    fn padded_buffer() -> Vec<u8> {
        let mut buffer = Vec::new();
        for row in 0..6_u8 {
            buffer.extend((0..6_u8).map(|column| row * 10 + column));
        }
        for plane in [100_u8, 200] {
            for row in 0..3_u8 {
                buffer.extend((0..3_u8).map(|column| plane + row * 10 + column));
            }
        }
        buffer
    }

    #[test]
    fn the_visible_picture_is_cut_out_of_a_padded_buffer() {
        let layout = BufferLayout {
            stride: 6,
            rows: 6,
            visible: Area {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
        };
        let packed = layout.copy_visible(&padded_buffer()).unwrap();
        assert_eq!(
            packed,
            [
                0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, //
                100, 101, 110, 111, //
                200, 201, 210, 211,
            ]
        );
    }

    #[test]
    fn an_offset_visible_area_moves_every_plane() {
        let layout = BufferLayout {
            stride: 6,
            rows: 6,
            visible: Area {
                x: 2,
                y: 2,
                width: 2,
                height: 2,
            },
        };
        let packed = layout.copy_visible(&padded_buffer()).unwrap();
        assert_eq!(packed, [22, 23, 32, 33, 111, 211]);
    }

    #[test]
    fn a_buffer_too_short_for_its_layout_is_refused() {
        let layout = BufferLayout {
            stride: 6,
            rows: 6,
            visible: Area {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
        };
        let mut buffer = padded_buffer();
        buffer.pop();
        assert!(matches!(
            layout.copy_visible(&buffer),
            Err(VideoError::UnsupportedFrame(_))
        ));
    }

    #[test]
    fn a_visible_area_outside_the_buffer_or_at_an_odd_position_is_refused() {
        let outside = BufferLayout {
            stride: 6,
            rows: 6,
            visible: Area {
                x: 4,
                y: 0,
                width: 4,
                height: 4,
            },
        };
        assert!(outside.copy_visible(&padded_buffer()).is_err());
        let odd = BufferLayout {
            stride: 6,
            rows: 6,
            visible: Area {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
        };
        assert!(odd.copy_visible(&padded_buffer()).is_err());
    }

    #[test]
    fn a_frame_exposes_its_packed_planes() {
        let frame = DecodedFrame::new(
            (4, 2),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited),
            ChromaSiting::MPEG2,
            FrameTiming {
                stream_time: Duration::ZERO,
                running_time: Duration::ZERO,
            },
        )
        .unwrap();
        let planes = frame.planes();
        assert_eq!(planes.planes[0].data, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(planes.planes[0].stride, 4);
        assert_eq!(planes.planes[1].data, [9, 10]);
        assert_eq!(planes.planes[2].data, [11, 12]);
        assert_eq!(planes.planes[2].stride, 2);
    }

    #[test]
    fn frame_data_of_the_wrong_size_is_refused() {
        let error = DecodedFrame::new(
            (2, 2),
            vec![0; 5],
            YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited),
            ChromaSiting::MPEG2,
            FrameTiming {
                stream_time: Duration::ZERO,
                running_time: Duration::ZERO,
            },
        );
        assert!(matches!(error, Err(VideoError::UnsupportedFrame(_))));
    }
}
