//! The video layer on the shared headless GPU device: what ends up in the pixels.

use super::video_layer::VideoLayer;
use crate::rendering::gpu_test::test_device;
use crate::video::{ChromaSiting, ColourMatrix, ColourRange, PlanarFrame, Plane, YuvToRgb};

/// The target is stored sRGB and drawn through its non-sRGB view, as the menu does.
const STORED: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const VIEWED: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// 4:3, so a 16:9 video gets bars above and below. A row is 256 bytes, the copy alignment.
const TARGET: (u32, u32) = (64, 48);
const VIDEO: (u32, u32) = (32, 18);
/// A 64 x 36 rectangle starting at row 6.
const BAR_ROWS: u32 = 6;

fn conversion() -> YuvToRgb {
    YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited)
}

struct Solid {
    luma: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    luma_stride: usize,
    chroma_stride: usize,
    size: (u32, u32),
}

impl Solid {
    /// A frame of one colour whose rows are padded to `padding` extra bytes of junk.
    fn new(size: (u32, u32), ycbcr: [u8; 3], padding: usize) -> Self {
        let chroma = (size.0.div_ceil(2) as usize, size.1.div_ceil(2) as usize);
        let plane = |value: u8, width: usize, height: usize| {
            let stride = width + padding;
            let mut data = vec![0xAB; stride * height];
            for row in data.chunks_mut(stride) {
                row[..width].fill(value);
            }
            data
        };
        Self {
            luma: plane(ycbcr[0], size.0 as usize, size.1 as usize),
            cb: plane(ycbcr[1], chroma.0, chroma.1),
            cr: plane(ycbcr[2], chroma.0, chroma.1),
            luma_stride: size.0 as usize + padding,
            chroma_stride: chroma.0 + padding,
            size,
        }
    }

    fn frame(&self) -> PlanarFrame<'_> {
        PlanarFrame {
            width: self.size.0,
            height: self.size.1,
            planes: [
                Plane {
                    data: &self.luma,
                    stride: self.luma_stride,
                },
                Plane {
                    data: &self.cb,
                    stride: self.chroma_stride,
                },
                Plane {
                    data: &self.cr,
                    stride: self.chroma_stride,
                },
            ],
        }
    }
}

struct Target {
    texture: wgpu::Texture,
}

impl Target {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video_layer_test_target"),
            size: wgpu::Extent3d {
                width: TARGET.0,
                height: TARGET.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: STORED,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[VIEWED],
        });
        Self { texture }
    }

    /// Clears to opaque dark blue, then draws the layer when it has something to draw.
    fn draw(&self, device: &wgpu::Device, queue: &wgpu::Queue, layer: &VideoLayer) {
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(VIEWED),
            ..Default::default()
        });
        let visible = layer.prepare(queue, TARGET);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if visible {
                layer.draw(&mut pass);
            }
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let bytes_per_row = TARGET.0 * 4;
        assert_eq!(bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video_layer_test_readback"),
            size: u64::from(bytes_per_row * TARGET.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(TARGET.1),
                },
            },
            wgpu::Extent3d {
                width: TARGET.0,
                height: TARGET.1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("receiver outlives the map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("video layer readback poll failed");
        receiver.recv().unwrap().expect("staging buffer maps");
        staging
            .slice(..)
            .get_mapped_range()
            .expect("staging range is mapped")
            .to_vec()
    }
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let start = ((y * TARGET.0 + x) * 4) as usize;
    [pixels[start], pixels[start + 1], pixels[start + 2]]
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], what: &str) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            a.abs_diff(e) <= 2,
            "{what}: {actual:?} is not close to {expected:?}"
        );
    }
}

const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn the_frame_fills_the_fitted_rectangle_and_leaves_the_bars_untouched() {
    let Some((device, queue)) = test_device("video layer fit") else {
        return;
    };
    let mut layer = VideoLayer::new(&device, VIEWED);
    let solid = Solid::new(VIDEO, [81, 90, 240], 0);
    layer.upload(
        &device,
        &queue,
        &solid.frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    let target = Target::new(&device);

    target.draw(&device, &queue, &layer);
    let pixels = target.pixels(&device, &queue);

    assert_close(at(&pixels, 32, 24), [255, 0, 0], "centre");
    assert_close(at(&pixels, 0, BAR_ROWS), [255, 0, 0], "first picture row");
    assert_close(
        at(&pixels, 63, TARGET.1 - BAR_ROWS - 1),
        [255, 0, 0],
        "last picture row",
    );
    assert_eq!(at(&pixels, 32, BAR_ROWS - 1), BLUE, "bar above");
    assert_eq!(at(&pixels, 32, TARGET.1 - BAR_ROWS), BLUE, "bar below");
}

#[test]
fn the_decoded_values_are_written_as_is_without_a_second_gamma_encoding() {
    let Some((device, queue)) = test_device("video layer gamma") else {
        return;
    };
    let mut layer = VideoLayer::new(&device, VIEWED);
    let grey_code = 126;
    let solid = Solid::new(VIDEO, [grey_code, 128, 128], 0);
    layer.upload(
        &device,
        &queue,
        &solid.frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    let target = Target::new(&device);

    target.draw(&device, &queue, &layer);

    let expected = conversion().apply([grey_code, 128, 128]);
    let expected = expected.map(|channel| (channel * 255.0).round() as u8);
    assert_close(
        at(&target.pixels(&device, &queue), 32, 24),
        expected,
        "grey",
    );
    assert!(
        expected[0].abs_diff(128) <= 1,
        "the fixture is a mid grey: {expected:?}"
    );
}

#[test]
fn row_padding_in_the_decoder_buffer_is_not_drawn() {
    let Some((device, queue)) = test_device("video layer stride") else {
        return;
    };
    let mut layer = VideoLayer::new(&device, VIEWED);
    let solid = Solid::new(VIDEO, [81, 90, 240], 16);
    layer.upload(
        &device,
        &queue,
        &solid.frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    let target = Target::new(&device);

    target.draw(&device, &queue, &layer);
    let pixels = target.pixels(&device, &queue);

    for x in [0, 31, 32, 63] {
        assert_close(at(&pixels, x, 24), [255, 0, 0], "padded row");
    }
}

#[test]
fn a_new_frame_replaces_the_old_one_even_at_another_size() {
    let Some((device, queue)) = test_device("video layer replace") else {
        return;
    };
    let mut layer = VideoLayer::new(&device, VIEWED);
    let target = Target::new(&device);
    layer.upload(
        &device,
        &queue,
        &Solid::new(VIDEO, [81, 90, 240], 0).frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    target.draw(&device, &queue, &layer);

    layer.upload(
        &device,
        &queue,
        &Solid::new((64, 36), [145, 54, 34], 0).frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    target.draw(&device, &queue, &layer);

    assert_close(
        at(&target.pixels(&device, &queue), 32, 24),
        [0, 255, 0],
        "second frame",
    );
}

#[test]
fn a_cleared_layer_draws_nothing() {
    let Some((device, queue)) = test_device("video layer clear") else {
        return;
    };
    let mut layer = VideoLayer::new(&device, VIEWED);
    layer.upload(
        &device,
        &queue,
        &Solid::new(VIDEO, [81, 90, 240], 0).frame(),
        conversion(),
        ChromaSiting::MPEG2,
    );
    assert!(layer.is_showing());
    layer.clear();
    assert!(!layer.is_showing());
    let target = Target::new(&device);

    target.draw(&device, &queue, &layer);

    assert_eq!(at(&target.pixels(&device, &queue), 32, 24), BLUE);
}

/// Cb grows by 12 per chroma sample, so a shifted lookup shows in the picture.
fn ramp_pixel(siting: ChromaSiting, target_x: u32) -> [u8; 3] {
    let (device, queue) = test_device("video layer chroma siting").expect("a GPU adapter");
    let mut solid = Solid::new(VIDEO, [126, 128, 128], 0);
    for row in solid.cb.chunks_mut(solid.chroma_stride) {
        for (x, cb) in row.iter_mut().take(16).enumerate() {
            *cb = 40 + 12 * x as u8;
        }
    }
    let mut layer = VideoLayer::new(&device, VIEWED);
    layer.upload(&device, &queue, &solid.frame(), conversion(), siting);
    let target = Target::new(&device);
    target.draw(&device, &queue, &layer);
    at(&target.pixels(&device, &queue), target_x, 24)
}

/// The blue channel a pixel gets for a Cb code that falls between two samples.
fn blue_for_cb(cb: f32) -> f32 {
    let conversion = conversion();
    let luma = 126.0 / 255.0 - conversion.bias[0];
    let chroma = cb / 255.0 - conversion.bias[1];
    let [_, _, blue_row] = conversion.rows.map(|row| row);
    ((blue_row[0] * luma + blue_row[1] * chroma) * 255.0).clamp(0.0, 255.0)
}

#[test]
fn a_left_sited_stream_reads_its_chroma_half_a_luma_pixel_to_the_right() {
    if test_device("video layer chroma siting probe").is_none() {
        return;
    }
    // Target column 32 is luma pixel 15.75: chroma sample 7.875 left-sited, 7.625 centred.
    let left_sited = ramp_pixel(ChromaSiting::MPEG2, 32)[2];
    let centred = ramp_pixel(
        ChromaSiting {
            horizontally_cosited: false,
            vertically_cosited: false,
        },
        32,
    )[2];
    assert!((f32::from(left_sited) - blue_for_cb(40.0 + 12.0 * 7.875)).abs() <= 1.5);
    assert!((f32::from(centred) - blue_for_cb(40.0 + 12.0 * 7.625)).abs() <= 1.5);
    assert!(left_sited > centred + 3);
}
