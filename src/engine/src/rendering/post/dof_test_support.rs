//! Fixtures and GPU plumbing shared by the tests of the depth of field passes.
//!
//! The passes run against the synthetic translated stand-ins of
//! `shaders::composed::dof::stubs`, which sum the registers and textures they read. The
//! tests therefore check the wiring (bindings, pixel coordinates, pass order, depth merge),
//! not the translated arithmetic.

use glam::{UVec2, Vec4};
use wgpu::util::DeviceExt;

use super::depth_of_field::DofRegisters;
use super::hdr_target::HDR_FORMAT;
/// Makes the pipelines of the passes compile without installed translated shaders.
pub use crate::rendering::shaders::startup::install_test_stubs;
use crate::rendering::water::ssr::tests::harness::{
    depth_texture, extent, f16_bits_to_f32, f32_to_f16_bits, read_mip,
};
use crate::rendering::water::textures::SURFACE_DEPTH_FORMAT;

/// Binary fractions, exact in the half-float scene targets.
pub fn probe_registers() -> DofRegisters {
    DofRegisters {
        cb2: [
            Vec4::ZERO,
            Vec4::new(0.125, 0.0625, 0.25, 0.5),
            Vec4::ZERO,
            Vec4::new(0.5, 0.25, 0.125, 0.0625),
            Vec4::new(1.0, 0.5, 0.25, 0.125),
            Vec4::ZERO,
        ],
    }
}

pub fn params_buffer(device: &wgpu::Device, registers: &DofRegisters) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dof_test_params"),
        contents: bytemuck::bytes_of(registers),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

/// What a one channel float texture holds at a texel as a four channel load.
pub fn depth_texel(raw_depth: f32) -> Vec4 {
    Vec4::new(raw_depth, 0.0, 0.0, 1.0)
}

pub fn view_of(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

pub fn hdr_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: UVec2,
    texels: &[Vec4],
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    assert_eq!(texels.len(), (size.x * size.y) as usize);
    let bytes: Vec<u8> = texels
        .iter()
        .flat_map(Vec4::to_array)
        .flat_map(|channel| f32_to_f16_bits(channel).to_le_bytes())
        .collect();
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("dof_test_hdr"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &bytes,
    )
}

pub fn read_hdr(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: UVec2,
) -> Vec<Vec4> {
    read_mip(device, queue, texture, 0, size, 8)
        .chunks_exact(8)
        .map(|texel| {
            let channel = |index: usize| {
                f16_bits_to_f32(u16::from_le_bytes([texel[index * 2], texel[index * 2 + 1]]))
            };
            Vec4::new(channel(0), channel(1), channel(2), channel(3))
        })
        .collect()
}

pub fn read_depth(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: UVec2,
) -> Vec<f32> {
    read_mip(device, queue, texture, 0, size, 4)
        .chunks_exact(4)
        .map(|texel| f32::from_le_bytes([texel[0], texel[1], texel[2], texel[3]]))
        .collect()
}

/// A single channel float texture of raw reverse-Z depths: the underwater surface depth, or
/// the merged depth the passes read.
pub fn float_depth_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: UVec2,
    raw_depth: &[f32],
) -> wgpu::Texture {
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("dof_test_float_depth"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SURFACE_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(raw_depth),
    )
}

pub fn scene_depth_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: UVec2,
    raw_depth: &[f32],
) -> wgpu::TextureView {
    view_of(&depth_texture(device, queue, size, raw_depth))
}
