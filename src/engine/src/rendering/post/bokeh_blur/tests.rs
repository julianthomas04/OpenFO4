#![allow(
    clippy::float_cmp,
    reason = "every value is a binary fraction, exact through the half-float targets"
)]

//! The blur chain's wiring on the GPU against the synthetic pass stand-ins: pass order,
//! the pixel each pass reads and the target the chain leaves behind.

use glam::{UVec2, Vec4};

use super::*;
use crate::rendering::{
    gpu_test::test_device,
    post::dof_test_support::{
        depth_texel, float_depth_texture, hdr_texture, install_test_stubs, params_buffer,
        probe_registers, read_hdr, view_of,
    },
};

const SIZE: UVec2 = UVec2::new(8, 4);

fn pixels() -> impl Iterator<Item = (u32, u32)> {
    (0..SIZE.y).flat_map(|y| (0..SIZE.x).map(move |x| (x, y)))
}

fn scene_texel((x, y): (u32, u32)) -> Vec4 {
    Vec4::new(x as f32 / 16.0, y as f32 / 16.0, 0.25, 1.0)
}

fn depth_at((x, y): (u32, u32)) -> f32 {
    (1 + x + y * SIZE.x) as f32 / 64.0
}

#[test]
fn the_chain_reads_each_pixel_through_all_three_passes() {
    install_test_stubs();
    let Some((device, queue)) = test_device("bokeh blur chain") else {
        return;
    };
    let blur = BokehBlur::new(&device, SIZE.x, SIZE.y);
    let sharp = hdr_texture(
        &device,
        &queue,
        SIZE,
        &pixels().map(scene_texel).collect::<Vec<_>>(),
        wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depths: Vec<f32> = pixels().map(depth_at).collect();
    let depth = float_depth_texture(&device, &queue, SIZE, &depths);
    let registers = probe_registers();
    let params = params_buffer(&device, &registers);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blur.encode(
        &device,
        &mut encoder,
        &SceneInputs {
            params: &params,
            sharp: &view_of(&sharp),
            merged_depth: &view_of(&depth),
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let actual = read_hdr(&device, &queue, blur.blurred_texture(), SIZE);
    let [_, blur_step, _, depth_registers, ..] = registers.cb2;
    for (pixel, texel) in pixels().zip(actual) {
        let first_pass = depth_registers + scene_texel(pixel) + depth_texel(depth_at(pixel));
        let expected = first_pass + blur_step + blur_step;
        assert_eq!(texel, expected, "pixel {pixel:?}");
    }
}

#[test]
fn the_blur_target_is_the_scene_size() {
    install_test_stubs();
    let Some((device, _)) = test_device("bokeh blur targets") else {
        return;
    };
    let mut blur = BokehBlur::new(&device, SIZE.x, SIZE.y);
    assert_eq!(blur.blurred_texture().size().width, SIZE.x);
    blur.resize(&device, 8, 6);
    assert_eq!(
        (
            blur.blurred_texture().size().width,
            blur.blurred_texture().size().height
        ),
        (8, 6)
    );
}
