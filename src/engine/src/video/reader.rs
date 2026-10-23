//! Reads the decoded frames of a video file through a Media Foundation source reader. The decoder
//! is asked for I420 straight away, so no colour converter sits between it and the engine.

use std::mem::ManuallyDrop;
use std::path::Path;
use std::time::Duration;

use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer2, IMFAttributes, IMFMediaBuffer, IMFSample, IMFSourceReader,
    MF_E_ATTRIBUTENOTFOUND, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE, MF_MT_VIDEO_CHROMA_SITING,
    MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF2DBuffer_LockFlags_Read,
    MFMediaType_Video, MFNominalRange_0_255, MFNominalRange_16_235, MFVideoArea,
    MFVideoChromaSubsampling_Horizontally_Cosited, MFVideoChromaSubsampling_Vertically_Cosited,
    MFVideoFormat_I420, MFVideoTransferMatrix_BT601, MFVideoTransferMatrix_BT709,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, Interface};

use super::colour::{ChromaSiting, ColourError, ColourMatrix, ColourRange, YuvToRgb};
use super::error::VideoError;
use super::frame::{Area, BufferLayout};
use super::media_foundation::{Session, failed};

/// Media Foundation counts time in 100-nanosecond ticks.
const TICK_NANOS: u64 = 100;
const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

/// A decoded frame, copied out of the decoder's buffer into tightly packed I420.
pub struct VideoSample {
    pub stream_time: Duration,
    pub duration: Duration,
    pub data: Vec<u8>,
}

/// What the decoder delivers and how to read it.
#[derive(Debug, Clone, Copy)]
pub struct Format {
    pub size: (u32, u32),
    pub conversion: YuvToRgb,
    pub siting: ChromaSiting,
    layout: BufferLayout,
    frame_period: Duration,
}

pub struct VideoReader {
    reader: IMFSourceReader,
    format: Format,
}

impl VideoReader {
    pub fn open(session: &Session, path: &Path) -> Result<Self, VideoError> {
        let reader = session.open(path)?;
        // SAFETY: plain calls on a live source reader.
        unsafe {
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(failed("deselect the streams"))?;
            reader
                .SetStreamSelection(VIDEO_STREAM, true)
                .map_err(failed("select the video stream"))?;
            let wanted = session.create_media_type()?;
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(failed("describe the wanted output"))?;
            wanted
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_I420)
                .map_err(failed("describe the wanted output"))?;
            reader
                .SetCurrentMediaType(VIDEO_STREAM, None, &wanted)
                .map_err(failed("decode the video to I420 (is it H.264?)"))?;
        }
        let format = read_format(&reader)?;
        Ok(Self { reader, format })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// The next frame, or `None` at the end of the file.
    pub fn read(&mut self) -> Result<Option<VideoSample>, VideoError> {
        loop {
            let mut flags = 0_u32;
            let mut timestamp = 0_i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: every out pointer points at a live local.
            unsafe {
                self.reader.ReadSample(
                    VIDEO_STREAM,
                    0,
                    None,
                    Some(&raw mut flags),
                    Some(&raw mut timestamp),
                    Some(&raw mut sample),
                )
            }
            .map_err(failed("decode the next frame"))?;
            let flags = flags as i32;
            if flags & MF_SOURCE_READERF_ERROR.0 != 0 {
                return Err(VideoError::MediaFoundation {
                    action: "decode the next frame",
                    reason: "the source reader reported a stream error".into(),
                });
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 != 0 {
                self.format = read_format(&self.reader)?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 != 0 {
                return Ok(None);
            }
            if let Some(sample) = sample {
                return self.copy(&sample, timestamp).map(Some);
            }
        }
    }

    /// Moves to `position` in the file; the next frame is the first one the decoder produces
    /// from there.
    pub fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        let ticks = i64::try_from(position.as_nanos() / u128::from(TICK_NANOS))
            .map_err(|_| VideoError::UnsupportedFrame(format!("cannot seek to {position:?}")))?;
        let target = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_I8,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 { hVal: ticks },
                }),
            },
        };
        // SAFETY: an all-zero GUID asks for 100-nanosecond units; `target` holds a plain integer
        // that needs no clearing.
        unsafe {
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &raw const target)
        }
        .map_err(failed("seek in the video"))
    }

    fn copy(&self, sample: &IMFSample, timestamp: i64) -> Result<VideoSample, VideoError> {
        let stream_time = ticks_to_duration(timestamp)?;
        // SAFETY: plain calls on a live sample.
        let duration = unsafe { sample.GetSampleDuration() }
            .ok()
            .filter(|&ticks| ticks > 0)
            .map_or(Ok(self.format.frame_period), ticks_to_duration)?;
        // SAFETY: as above.
        let buffer =
            unsafe { sample.ConvertToContiguousBuffer() }.map_err(failed("read a frame"))?;
        let data = match buffer.cast::<IMF2DBuffer2>() {
            Ok(two_dimensional) => copy_2d(&two_dimensional, self.format.layout)?,
            Err(_) => copy_contiguous(&buffer, self.format.layout)?,
        };
        Ok(VideoSample {
            stream_time,
            duration,
            data,
        })
    }
}

/// Copies from a buffer that reports its own pitch.
fn copy_2d(buffer: &IMF2DBuffer2, layout: BufferLayout) -> Result<Vec<u8>, VideoError> {
    let mut scanline = std::ptr::null_mut();
    let mut pitch = 0_i32;
    let mut start = std::ptr::null_mut();
    let mut length = 0_u32;
    // SAFETY: every out pointer points at a live local; unlocked below.
    unsafe {
        buffer.Lock2DSize(
            MF2DBuffer_LockFlags_Read,
            &raw mut scanline,
            &raw mut pitch,
            &raw mut start,
            &raw mut length,
        )
    }
    .map_err(failed("lock a frame"))?;
    let copied = usize::try_from(pitch)
        .map_err(|_| VideoError::UnsupportedFrame(format!("bottom-up frame (pitch {pitch})")))
        .and_then(|stride| {
            // SAFETY: the lock maps `length` bytes from `start`, and the picture begins at
            // `scanline` inside them.
            let bytes = unsafe {
                let offset = scanline.offset_from(start).unsigned_abs();
                std::slice::from_raw_parts(scanline, (length as usize).saturating_sub(offset))
            };
            BufferLayout { stride, ..layout }.copy_visible(bytes)
        });
    // SAFETY: balances the lock above.
    unsafe { buffer.Unlock2D() }.map_err(failed("unlock a frame"))?;
    copied
}

/// Copies from a plain buffer laid out with the media type's stride.
fn copy_contiguous(buffer: &IMFMediaBuffer, layout: BufferLayout) -> Result<Vec<u8>, VideoError> {
    let mut data = std::ptr::null_mut();
    let mut length = 0_u32;
    // SAFETY: the out pointers point at live locals; unlocked below.
    unsafe { buffer.Lock(&raw mut data, None, Some(&raw mut length)) }
        .map_err(failed("lock a frame"))?;
    // SAFETY: the lock maps `length` bytes from `data`.
    let copied = layout.copy_visible(unsafe { std::slice::from_raw_parts(data, length as usize) });
    // SAFETY: balances the lock above.
    unsafe { buffer.Unlock() }.map_err(failed("unlock a frame"))?;
    copied
}

fn ticks_to_duration(ticks: i64) -> Result<Duration, VideoError> {
    u64::try_from(ticks)
        .map(|ticks| Duration::from_nanos(ticks * TICK_NANOS))
        .map_err(|_| VideoError::UnsupportedFrame(format!("negative timestamp {ticks}")))
}

fn read_format(reader: &IMFSourceReader) -> Result<Format, VideoError> {
    // SAFETY: plain calls on a live source reader.
    let current = unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }
        .map_err(failed("read the output format"))?;
    let attributes: &IMFAttributes = &current;
    let subtype = required(unsafe { attributes.GetGUID(&MF_MT_SUBTYPE) })?;
    if subtype != MFVideoFormat_I420 {
        return Err(VideoError::UnsupportedFrame(format!(
            "the decoder delivers {subtype:?} instead of I420"
        )));
    }
    let (coded_width, coded_height) = split(required(unsafe {
        attributes.GetUINT64(&MF_MT_FRAME_SIZE)
    })?);
    let (rate_numerator, rate_denominator) = split(required(unsafe {
        attributes.GetUINT64(&MF_MT_FRAME_RATE)
    })?);
    if rate_numerator == 0 || rate_denominator == 0 {
        return Err(VideoError::UnsupportedFrame(format!(
            "frame rate {rate_numerator}/{rate_denominator}"
        )));
    }
    let stride = optional(unsafe { attributes.GetUINT32(&MF_MT_DEFAULT_STRIDE) })?
        .map_or(coded_width, |stride| stride.cast_signed().unsigned_abs());
    let visible = aperture(attributes)?.unwrap_or(Area {
        x: 0,
        y: 0,
        width: coded_width,
        height: coded_height,
    });
    // SAFETY: as above.
    let stream = unsafe { reader.GetNativeMediaType(VIDEO_STREAM, 0) }
        .map_err(failed("read the stream format"))?;
    // The decoder may leave the colour description out of its output; the stream carries it.
    let declared = |key: &GUID| match optional(unsafe { attributes.GetUINT32(key) })? {
        Some(value) => Ok(Some(value)),
        None => optional(unsafe { stream.GetUINT32(key) }),
    };
    let colour = |error: ColourError| VideoError::UnsupportedFrame(error.to_string());
    let conversion = YuvToRgb::new(
        colour_matrix(declared(&MF_MT_YUV_MATRIX)?).map_err(colour)?,
        colour_range(declared(&MF_MT_VIDEO_NOMINAL_RANGE)?).map_err(colour)?,
    );
    let siting = chroma_siting(declared(&MF_MT_VIDEO_CHROMA_SITING)?);
    Ok(Format {
        size: (visible.width, visible.height),
        conversion,
        siting,
        layout: BufferLayout {
            stride: stride as usize,
            rows: coded_height as usize,
            visible,
        },
        frame_period: Duration::from_nanos(
            1_000_000_000 * u64::from(rate_denominator) / u64::from(rate_numerator),
        ),
    })
}

/// The part of the coded picture meant to be shown, when the format names one.
fn aperture(attributes: &IMFAttributes) -> Result<Option<Area>, VideoError> {
    let mut area = MFVideoArea::default();
    // SAFETY: `area` is plain data the blob is copied into, and the slice covers exactly it.
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(
            (&raw mut area).cast::<u8>(),
            std::mem::size_of::<MFVideoArea>(),
        )
    };
    let Some(()) =
        optional(unsafe { attributes.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, bytes, None) })?
    else {
        return Ok(None);
    };
    let whole = |offset: windows::Win32::Media::MediaFoundation::MFOffset| {
        u32::try_from(offset.value)
            .ok()
            .filter(|_| offset.fract == 0)
            .ok_or_else(|| VideoError::UnsupportedFrame(format!("aperture offset {offset:?}")))
    };
    let size = |value: i32| {
        u32::try_from(value)
            .map_err(|_| VideoError::UnsupportedFrame(format!("aperture size {value}")))
    };
    Ok(Some(Area {
        x: whole(area.OffsetX)?,
        y: whole(area.OffsetY)?,
        width: size(area.Area.cx)?,
        height: size(area.Area.cy)?,
    }))
}

fn split(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

fn required<T>(value: windows::core::Result<T>) -> Result<T, VideoError> {
    value.map_err(failed("read the output format"))
}

/// An attribute the format may leave out.
fn optional<T>(value: windows::core::Result<T>) -> Result<Option<T>, VideoError> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code() == MF_E_ATTRIBUTENOTFOUND => Ok(None),
        Err(error) => Err(failed("read the output format")(error)),
    }
}

fn colour_matrix(declared: Option<u32>) -> Result<ColourMatrix, ColourError> {
    match declared.map(u32::cast_signed) {
        Some(value) if value == MFVideoTransferMatrix_BT601.0 => Ok(ColourMatrix::Bt601),
        Some(value) if value == MFVideoTransferMatrix_BT709.0 => Ok(ColourMatrix::Bt709),
        other => Err(ColourError::UnknownMatrix(format!("{other:?}"))),
    }
}

fn colour_range(declared: Option<u32>) -> Result<ColourRange, ColourError> {
    match declared.map(u32::cast_signed) {
        Some(value) if value == MFNominalRange_16_235.0 => Ok(ColourRange::Limited),
        Some(value) if value == MFNominalRange_0_255.0 => Ok(ColourRange::Full),
        other => Err(ColourError::UnknownRange(format!("{other:?}"))),
    }
}

/// The siting the stream declares; one that declares none gets the H.264 and MPEG-2 default.
fn chroma_siting(declared: Option<u32>) -> ChromaSiting {
    match declared.map(u32::cast_signed) {
        None | Some(0) => ChromaSiting::MPEG2,
        Some(flags) => ChromaSiting {
            horizontally_cosited: flags & MFVideoChromaSubsampling_Horizontally_Cosited.0 != 0,
            vertically_cosited: flags & MFVideoChromaSubsampling_Vertically_Cosited.0 != 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::MediaFoundation::{
        MFVideoChromaSubsampling_Cosited, MFVideoChromaSubsampling_MPEG1,
        MFVideoChromaSubsampling_MPEG2, MFVideoTransferMatrix_BT2020_10,
    };

    #[test]
    fn the_declared_matrix_and_range_are_used() {
        let raw = |value: i32| Some(value.cast_unsigned());
        assert_eq!(
            colour_matrix(raw(MFVideoTransferMatrix_BT601.0)),
            Ok(ColourMatrix::Bt601)
        );
        assert_eq!(
            colour_matrix(raw(MFVideoTransferMatrix_BT709.0)),
            Ok(ColourMatrix::Bt709)
        );
        assert_eq!(
            colour_range(raw(MFNominalRange_16_235.0)),
            Ok(ColourRange::Limited)
        );
        assert_eq!(
            colour_range(raw(MFNominalRange_0_255.0)),
            Ok(ColourRange::Full)
        );
    }

    #[test]
    fn an_undeclared_or_unsupported_matrix_or_range_is_refused() {
        assert!(colour_matrix(None).is_err());
        assert!(colour_matrix(Some(MFVideoTransferMatrix_BT2020_10.0.cast_unsigned())).is_err());
        assert!(colour_range(None).is_err());
        assert!(colour_range(Some(0)).is_err());
    }

    #[test]
    fn the_declared_chroma_site_is_used_and_an_undeclared_one_is_left_sited() {
        let raw = |value: i32| Some(value.cast_unsigned());
        assert_eq!(chroma_siting(None), ChromaSiting::MPEG2);
        assert_eq!(chroma_siting(Some(0)), ChromaSiting::MPEG2);
        assert_eq!(
            chroma_siting(raw(MFVideoChromaSubsampling_MPEG2.0)),
            ChromaSiting::MPEG2
        );
        let centred = chroma_siting(raw(MFVideoChromaSubsampling_MPEG1.0));
        assert!(!centred.horizontally_cosited && !centred.vertically_cosited);
        let top_left = chroma_siting(raw(MFVideoChromaSubsampling_Cosited.0));
        assert!(top_left.horizontally_cosited && top_left.vertically_cosited);
    }

    #[test]
    fn a_packed_size_splits_into_high_and_low_halves() {
        assert_eq!(split((1920 << 32) + 1088), (1920, 1088));
    }
}
