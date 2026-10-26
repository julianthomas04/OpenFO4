//! Test support: the one headless GPU device every GPU test shares, the readback of what a test
//! drew and the conversions of the values it seeds and reads; the translated shaders are the
//! synthetic stubs.
//!
//! A device per test means an instance per test, and tearing instances down while others are
//! still being created hangs the driver on Windows. `Device` and `Queue` are `Send + Sync`
//! handles and error scopes are per thread, so tests share the device without seeing each
//! other's errors.
//!
//! The device also carries whichever production features the adapter offers, so shaders the
//! game compiles with those features (the binding arrays of the object shader) compile in a
//! test too.

use std::sync::OnceLock;

use glam::UVec2;

use crate::rendering::{
    cube_array::CUBE_SLOTS,
    device::{REQUIRED_FEATURES, STORAGE_BUFFERS_PER_STAGE},
    gpu_driven::MAX_TEXTURES,
    instance,
    water::displacement::REQUIRED_FORMAT_FEATURES,
};

const REQUIRE_GPU_ENV: &str = "OPENFO4_REQUIRE_GPU_TESTS";

/// The 2D and cube textures of the world pass material group.
const BINDLESS_TEXTURES: u32 = MAX_TEXTURES + CUBE_SLOTS;

static SHARED_DEVICE: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();

/// The shared device and queue, or `None` after logging which test was skipped. Setting
/// `OPENFO4_REQUIRE_GPU_TESTS=1` turns the skip into a failure (for CI runners that are
/// meant to have an adapter).
pub fn test_device(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    crate::rendering::shaders::startup::install_test_stubs();
    match SHARED_DEVICE.get_or_init(create_device) {
        Ok((device, queue)) => Some((device.clone(), queue.clone())),
        Err(reason) => skipped(test, reason),
    }
}

/// The shared device for the tests that build the world pass layouts, whose material group is
/// the bindless texture array: `None` after logging the skip when the adapter lacks a feature
/// or the room that group needs.
pub fn world_pass_device(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let (device, queue) = test_device(test)?;
    match world_pass_shortfall(&device) {
        Some(reason) => skipped(test, &reason),
        None => Some((device, queue)),
    }
}

fn skipped<T>(test: &str, reason: &str) -> Option<T> {
    assert!(
        std::env::var_os(REQUIRE_GPU_ENV).is_none(),
        "{REQUIRE_GPU_ENV} is set but the GPU test `{test}` cannot run: {reason}"
    );
    eprintln!("SKIPPED GPU test `{test}`: {reason}");
    None
}

fn world_pass_shortfall(device: &wgpu::Device) -> Option<String> {
    let missing = REQUIRED_FEATURES - device.features();
    if !missing.is_empty() {
        return Some(format!("the adapter lacks the device features {missing:?}"));
    }
    let limits = device.limits();
    (limits.max_sampled_textures_per_shader_stage < BINDLESS_TEXTURES
        || limits.max_binding_array_elements_per_shader_stage < BINDLESS_TEXTURES)
        .then(|| format!("the adapter offers fewer than {BINDLESS_TEXTURES} bindless textures"))
}

fn create_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let adapter = pollster::block_on(instance::new().request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        },
    ))
    .map_err(|error| format!("no adapter: {error}"))?;
    let adapter_limits = adapter.limits();
    let limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: STORAGE_BUFFERS_PER_STAGE
            .min(adapter_limits.max_storage_buffers_per_shader_stage),
        max_sampled_textures_per_shader_stage: BINDLESS_TEXTURES
            .min(adapter_limits.max_sampled_textures_per_shader_stage),
        max_binding_array_elements_per_shader_stage: BINDLESS_TEXTURES
            .min(adapter_limits.max_binding_array_elements_per_shader_stage),
        ..Default::default()
    };
    let descriptor = wgpu::DeviceDescriptor {
        required_features: REQUIRED_FORMAT_FEATURES | (adapter.features() & REQUIRED_FEATURES),
        required_limits: limits,
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&descriptor))
        .map_err(|error| format!("no device: {error}"))
}

/// The extent of a single-layer 2D texture of `size`.
pub fn extent(size: UVec2) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    }
}

/// Reads one mip of a texture back, rows unpadded.
pub fn read_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    size: UVec2,
    bytes_per_pixel: u32,
) -> Vec<u8> {
    let row_bytes = size.x * bytes_per_pixel;
    let padded_row = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_readback"),
        size: u64::from(padded_row * size.y),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(size.y),
            },
        },
        extent(size),
    );
    queue.submit(std::iter::once(encoder.finish()));
    let data = map_read(device, &buffer);
    (0..size.y as usize)
        .flat_map(|row| {
            let start = row * padded_row as usize;
            data[start..start + row_bytes as usize].to_vec()
        })
        .collect()
}

/// The texels of one mip of a texture of 32-bit floats.
pub fn read_f32_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    size: UVec2,
) -> Vec<f32> {
    bytemuck::cast_slice(&read_mip(device, queue, texture, mip, size, 4)).to_vec()
}

/// The bytes of `buffer`, once the GPU is done with it.
pub fn map_read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender
                .send(result)
                .expect("the test outlives its map callback");
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("readback poll");
    receiver
        .recv()
        .expect("the poll runs the map callback")
        .expect("readback map");
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("map the readback buffer")
        .to_vec();
    buffer.unmap();
    data
}

/// `value` as IEEE half-float bits, rounded to nearest even. Finite values only.
pub fn f32_to_f16_bits(value: f32) -> u16 {
    assert!(value.is_finite(), "{value} has no half-float form here");
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x007f_ffff;
    if exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let full = mantissa | 0x0080_0000;
        let shift = (14 - exponent) as u32;
        let kept = full >> shift;
        let remainder = full & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        let rounded =
            kept + u32::from(remainder > halfway || (remainder == halfway && kept & 1 == 1));
        return sign | rounded as u16;
    }
    let kept = mantissa >> 13;
    let remainder = mantissa & 0x1fff;
    let combined = ((exponent as u32) << 10) | kept;
    let rounded =
        combined + u32::from(remainder > 0x1000 || (remainder == 0x1000 && combined & 1 == 1));
    sign | rounded as u16
}

/// The half float whose IEEE bits are `bits`.
pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        0x1f => sign * f32::INFINITY,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// The value as a half float would store it.
pub fn through_f16(value: f32) -> f32 {
    f16_bits_to_f32(f32_to_f16_bits(value))
}

pub fn srgb_byte_to_linear(byte: u8) -> f32 {
    let encoded = f32::from(byte) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb_byte(linear: f32) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_round_trip_the_values_the_tests_seed() {
        for value in [0.0_f32, 1.0, -1.0, 0.5, 0.25, 2.0, 100.0, -47.5, 0.001] {
            let rounded = through_f16(value);
            assert!(
                (rounded - value).abs() <= value.abs() * 1e-3 + 1e-6,
                "{value} -> {rounded}"
            );
        }
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(-2.0), 0xc000);
        assert_eq!(f32_to_f16_bits(65_504.0), 0x7bff);
        assert_eq!(f16_bits_to_f32(0x3555), 0.333_251_95);
    }

    #[test]
    fn srgb_bytes_round_trip() {
        for byte in 0..=255u8 {
            assert_eq!(linear_to_srgb_byte(srgb_byte_to_linear(byte)), byte);
        }
    }
}
