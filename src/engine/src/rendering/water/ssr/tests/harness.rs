//! GPU test plumbing: texture seeding and readback, half-float conversion, and a
//! depth buffer filled from explicit values.

use glam::UVec2;
use wgpu::util::DeviceExt;

use crate::rendering::texture::Texture;

pub fn extent(size: UVec2) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    }
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
        label: Some("ssr_test_readback"),
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

pub fn read_f32_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    size: UVec2,
) -> Vec<f32> {
    bytemuck::cast_slice(&read_mip(device, queue, texture, mip, size, 4)).to_vec()
}

const DEPTH_FILL: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

struct Output {
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> Output {
    return Output(textureLoad(source, vec2<i32>(position.xy), 0).x);
}
";

/// A `Depth32Float` texture holding `raw_depth` (row-major, one value per
/// texel). The depth format cannot be uploaded to, so a pass writes it.
pub fn depth_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: UVec2,
    raw_depth: &[f32],
) -> wgpu::Texture {
    let source = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("ssr_test_depth_source"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(raw_depth),
    );
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ssr_test_depth"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: Texture::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ssr_test_depth_fill"),
        source: wgpu::ShaderSource::Wgsl(DEPTH_FILL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ssr_test_depth_fill_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ssr_test_depth_fill"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            targets: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ssr_test_depth_fill_group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(
                &source.create_view(&wgpu::TextureViewDescriptor::default()),
            ),
        }],
    });
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ssr_test_depth_fill_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(std::iter::once(encoder.finish()));
    depth
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
