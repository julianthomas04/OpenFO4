//! A tiny FLAC writer for tests: one mono 16-bit sine frame stored with a verbatim subframe.
//! The sine rides on an offset that keeps every sample positive: a sample byte pair that looked
//! like a frame sync code would be taken for a frame boundary by the decoder's packet parser.

use std::f32::consts::TAU;

const SINE_HZ: f32 = 440.0;
const SINE_AMPLITUDE: f32 = 8000.0;
const SINE_OFFSET: f32 = 12000.0;
const VERBATIM_SUBFRAME: u8 = 0x02;
const RESERVED_SUBFRAME: u8 = 0x04;

fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |mut crc, byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |mut crc, byte| {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                crc << 1 ^ 0x8005
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// A valid FLAC file of `sample_count` mono samples (at most 65536) of a sine wave.
pub fn sine_flac(sample_rate: u32, sample_count: u32) -> Vec<u8> {
    flac_with_subframe(sample_rate, sample_count, VERBATIM_SUBFRAME)
}

/// A FLAC file whose header and frame header are valid and whose subframe type is reserved, so
/// it opens but cannot be decoded.
pub fn undecodable_flac(sample_rate: u32, sample_count: u32) -> Vec<u8> {
    flac_with_subframe(sample_rate, sample_count, RESERVED_SUBFRAME)
}

fn flac_with_subframe(sample_rate: u32, sample_count: u32, subframe_header: u8) -> Vec<u8> {
    let samples: Vec<i16> = (0..sample_count)
        .map(|index| {
            (SINE_OFFSET
                + SINE_AMPLITUDE * (TAU * SINE_HZ * index as f32 / sample_rate as f32).sin())
                as i16
        })
        .collect();

    let mut frame = vec![0xFF, 0xF8, 0x70, 0x08, 0x00];
    frame.extend(((sample_count - 1) as u16).to_be_bytes());
    frame.push(crc8(&frame));
    frame.push(subframe_header);
    frame.extend(samples.iter().flat_map(|sample| sample.to_be_bytes()));
    frame.extend(crc16(&frame).to_be_bytes());

    let frame_size = (frame.len() as u32).to_be_bytes();
    let mut file = b"fLaC".to_vec();
    file.extend([0x80, 0, 0, 34]);
    file.extend((sample_count as u16).to_be_bytes());
    file.extend((sample_count as u16).to_be_bytes());
    file.extend(&frame_size[1..]);
    file.extend(&frame_size[1..]);
    let packed = u64::from(sample_rate) << 44 | 15 << 36 | u64::from(sample_count);
    file.extend(packed.to_be_bytes());
    file.extend([0; 16]);
    file.extend(frame);
    file
}
