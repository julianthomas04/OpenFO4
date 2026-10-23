//! GPU tests of the refraction stage on the shared test device: a card over a colour ramp.

#![allow(
    clippy::float_cmp,
    reason = "untouched pixels keep the exact clear value 0.0"
)]

use bytemuck::Zeroable;
use glam::{Mat4, Vec4};
use serde_json::json;

use super::*;
use crate::rendering::effects::shader::EffectVertex;
use crate::rendering::frustum::Aabb;
use crate::rendering::gpu_test::test_device;

const SIZE: u32 = 32;
const NEAR: f32 = 0.5;
/// Clip depth of the card: view depth `NEAR / 0.05 = 10` eu.
const CARD_DEPTH: f32 = 0.05;
const NORMAL_PATH: &str = "textures\\effects\\test_n.dds";
/// The offset limit in pixels: the strength of the test card exceeds it.
const OFFSET_PX: f32 = super::super::offset::MAX_OFFSET_UV * SIZE as f32;
/// Depth value (reverse-Z) of a surface much nearer than the card.
const NEAR_SURFACE_DEPTH: f32 = 0.5;

fn f16_bits(value: f32) -> u16 {
    if value == 0.0 {
        return 0;
    }
    let bits = value.to_bits();
    let exponent = (bits >> 23) as i32 - 127 + 15;
    ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
}

fn f16_value(bits: u16) -> f32 {
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    if exponent == 0 {
        mantissa * 2f32.powi(-24)
    } else {
        (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
    }
}

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("refraction_test"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn write(queue: &wgpu::Queue, target: &wgpu::Texture, bytes: &[u8], bytes_per_texel: u32) {
    queue.write_texture(
        target.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * bytes_per_texel),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
}

/// Scene colour: red ramps with x (`(x + 0.5) / SIZE`); the scene depth is the sky except in
/// `occluded_columns`, where a surface sits much nearer than the card.
fn scene_sources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    occluded_columns: std::ops::Range<u32>,
) -> (wgpu::Texture, wgpu::Texture) {
    let colour = texture(
        device,
        HDR_FORMAT,
        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
    );
    let texels: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|index| {
            let red = ((index % SIZE) as f32 + 0.5) / SIZE as f32;
            [red, 0.0, 0.0, 1.0]
                .map(|c| f16_bits(c).to_le_bytes())
                .concat()
        })
        .collect();
    write(queue, &colour, &texels, 8);
    let depth = texture(
        device,
        Texture::DEPTH_FORMAT,
        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    draw_depth_strip(device, queue, &depth, occluded_columns);
    (colour, depth)
}

/// Clears `depth` to the sky and writes [`NEAR_SURFACE_DEPTH`] into the pixel columns of
/// `columns` (depth textures cannot be written from the CPU).
fn draw_depth_strip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    depth: &wgpu::Texture,
    columns: std::ops::Range<u32>,
) {
    let ndc = |column: u32| column as f32 / SIZE as f32 * 2.0 - 1.0;
    let source = format!(
        "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
            let x = select({:.6}, {:.6}, (i & 1u) == 1u);
            let y = select(-1.0, 1.0, (i & 2u) == 2u);
            return vec4<f32>(x, y, {NEAR_SURFACE_DEPTH:.6}, 1.0);
        }}",
        ndc(columns.start),
        ndc(columns.end),
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("refraction_test_strip"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("refraction_test_strip"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
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
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("refraction_test_depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.create_view(&wgpu::TextureViewDescriptor::default()),
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
        if !columns.is_empty() {
            pass.set_pipeline(&pipeline);
            pass.draw(0..4, 0..1);
        }
    }
    queue.submit([encoder.finish()]);
}

fn card() -> RefractionInstance {
    let corner = |x: f32, y: f32| EffectVertex {
        position: [x, y, CARD_DEPTH],
        uv: [x + 0.5, y + 0.5],
        normal: [0.0, 0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        colour: [255; 4],
    };
    RefractionInstance {
        mesh: Arc::new(EffectMeshData::from_vertices(
            vec![
                corner(-0.5, -0.5),
                corner(0.5, -0.5),
                corner(0.5, 0.5),
                corner(-0.5, 0.5),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )),
        material: Arc::new(
            serde_json::from_value(json!({
                "v": 1, "strength": 0.1,
                "normal_a": "effects/test_n.dds", "normal_b": "effects/test_n.dds",
                "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
                "vertex_colors": true, "double_sided": true
            }))
            .unwrap(),
        ),
        model: Mat4::IDENTITY,
        bounds: Aabb {
            min: Vec3::new(-0.5, -0.5, CARD_DEPTH),
            max: Vec3::new(0.5, 0.5, CARD_DEPTH),
        },
    }
}

fn frame() -> FrameUniform {
    let size = SIZE as f32;
    let mut frame = FrameUniform::zeroed();
    frame.view_proj = Mat4::IDENTITY;
    frame.camera_pos = Vec4::new(0.0, 0.0, -5.0, 1.0);
    frame.camera_forward = Vec4::new(0.0, 0.0, 1.0, 0.0);
    frame.viewport = Vec4::new(size, size, 1.0 / size, 1.0 / size);
    frame
}

fn read_red(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    target: &wgpu::Texture,
) -> Vec<f32> {
    let bytes_per_row = SIZE * 8;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("refraction_test_readback"),
        size: u64::from(bytes_per_row * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device polls");
    let data = slice
        .get_mapped_range()
        .expect("the readback buffer is mapped");
    data.chunks_exact(8)
        .map(|texel| f16_value(u16::from_le_bytes([texel[0], texel[1]])))
        .collect()
}

/// An asset root holding the card's normal map: uniform, tilted towards +x, so the distortion
/// is a pure horizontal shift.
fn root_with_tilted_normal() -> tempfile::TempDir {
    use crate::assets::types::texture::cube_test_support::{DdsSpec, Format, Header};
    const DDS_HEADER_BYTES: usize = 128;
    let spec = DdsSpec {
        width: SIZE,
        height: SIZE,
        mips: 1,
        format: Format::Rgba8,
        header: Header::Legacy,
        cube: false,
        face_flags: false,
        drop_tail_bytes: 0,
    };
    let mut file = spec.into_bytes();
    file.truncate(DDS_HEADER_BYTES);
    file.extend((0..SIZE * SIZE).flat_map(|_| [255, 128, 255, 255]));
    let root = tempfile::TempDir::new().unwrap();
    let path = root.path().join(NORMAL_PATH);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, file).unwrap();
    root
}

/// Renders the card over the scene and returns the red channel of every pixel.
fn render_red(occluded_columns: std::ops::Range<u32>) -> Option<Vec<f32>> {
    let (device, queue) = test_device("refraction card")?;
    let scene = SceneCopy::new(&device, SIZE, SIZE);
    let root = root_with_tilted_normal();
    let mut stage = RefractionStage::new(
        &device,
        &queue,
        scene.layout(),
        root.path().to_str().expect("the temp dir path is UTF-8"),
    );
    let (colour_source, depth_source) = scene_sources(&device, &queue, occluded_columns);
    let target = texture(
        &device,
        HDR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        &device,
        Texture::DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let instance = card();
    let frame = frame();

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let inputs = RefractionFrameInputs {
        frame: &frame,
        near_plane: NEAR,
        elapsed_seconds: 0.0,
    };
    stage.prepare(&inputs, |visit| visit(&instance));
    assert_eq!(stage.draw_count(), 0, "the card waits for its normal maps");
    stage.wait_for_warmup();
    stage.prepare(&inputs, |visit| visit(&instance));
    assert_eq!(stage.draw_count(), 1);
    assert!(stage.has_work());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    scene.encode(&mut encoder, &colour_source, &depth_source);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("refraction_test_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.create_view(&wgpu::TextureViewDescriptor::default()),
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.create_view(&wgpu::TextureViewDescriptor::default()),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        assert!(stage.draw(&mut pass, &scene));
    }
    let red = read_red(&device, &queue, encoder, &target);
    assert!(pollster::block_on(scope.pop()).is_none());
    Some(red)
}

fn at(red: &[f32], x: u32, y: u32) -> f32 {
    red[(y * SIZE + x) as usize]
}

fn ramp(x: f32) -> f32 {
    (x + 0.5) / SIZE as f32
}

#[test]
fn card_pixels_show_the_scene_shifted_and_pixels_outside_stay_untouched() {
    let Some(red) = render_red(0..0) else { return };

    let inside = at(&red, 16, 16);
    let expected = ramp(16.0 + OFFSET_PX);
    assert!(
        (inside - expected).abs() < 0.01,
        "the pixel shows the scene {OFFSET_PX} px to the right: {inside} vs {expected}"
    );
    assert!(
        inside > ramp(16.0) + 0.05,
        "and so differs from the undistorted scene: {inside}"
    );
    assert_eq!(
        at(&red, 2, 16),
        0.0,
        "outside the card the target is untouched"
    );
    assert_eq!(at(&red, 28, 16), 0.0);
    assert_eq!(at(&red, 16, 2), 0.0);
}

#[test]
fn a_sample_in_front_of_the_card_is_rejected() {
    // Pixel 16 samples around pixel 18.5; the surface covering columns 18..20 is nearer than
    // the card, so that pixel must not be pulled in and the card leaves it alone.
    let Some(red) = render_red(18..20) else {
        return;
    };

    assert_eq!(at(&red, 16, 16), 0.0, "the rejected pixel keeps the target");
    assert!(
        at(&red, 9, 16) > 0.3,
        "a pixel whose sample is clear is still distorted: {}",
        at(&red, 9, 16)
    );
}

#[test]
fn a_card_whose_normal_map_cannot_be_loaded_is_marked_failed_and_not_asked_for_again() {
    let Some((device, queue)) = test_device("refraction missing normal") else {
        return;
    };
    let scene = SceneCopy::new(&device, SIZE, SIZE);
    let empty_root = tempfile::TempDir::new().unwrap();
    let mut stage = RefractionStage::new(
        &device,
        &queue,
        scene.layout(),
        empty_root.path().to_str().unwrap(),
    );
    let instance = card();
    let frame = frame();
    let inputs = RefractionFrameInputs {
        frame: &frame,
        near_plane: NEAR,
        elapsed_seconds: 0.0,
    };

    stage.prepare(&inputs, |visit| visit(&instance));
    stage.wait_for_warmup();
    stage.prepare(&inputs, |visit| visit(&instance));

    assert_eq!(stage.draw_count(), 0);
    assert!(stage.failed_materials.contains(&instance.material, ()));
    assert!(stage.pending_materials.is_empty());
}
