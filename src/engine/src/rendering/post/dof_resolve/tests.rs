#![allow(
    clippy::float_cmp,
    reason = "every value is a binary fraction, exact through the half-float targets"
)]

//! The final pass's wiring on the GPU against the synthetic pass stand-ins: which pixel of
//! the sharp, blurred and depth inputs each output pixel reads, and which variant runs.

use glam::{UVec2, Vec4};

use super::*;
use crate::rendering::{
    gpu_test::test_device,
    post::{
        depth_of_field::DofRegisters,
        dof_test_support::{
            depth_texel, float_depth_texture, hdr_texture, install_test_stubs, params_buffer,
            probe_registers, read_hdr, view_of,
        },
    },
};

const SIZE: UVec2 = UVec2::new(4, 4);
const BLURRED_SIZE: UVec2 = UVec2::new(2, 2);

fn sharp_texel(x: u32, y: u32) -> Vec4 {
    Vec4::new(x as f32 / 8.0, y as f32 / 8.0, 0.25, 1.0)
}

fn blurred_texel(x: u32, y: u32) -> Vec4 {
    Vec4::new(0.5 + x as f32 / 4.0, 0.5 + y as f32 / 4.0, 0.0, 0.5)
}

fn depth_at(x: u32, y: u32) -> f32 {
    (1 + x + y * SIZE.x) as f32 / 32.0
}

fn grid(size: UVec2, texel: impl Fn(u32, u32) -> Vec4) -> Vec<Vec4> {
    (0..size.y)
        .flat_map(|y| (0..size.x).map(move |x| (x, y)))
        .map(|(x, y)| texel(x, y))
        .collect()
}

fn run_pass(registers: &DofRegisters, variant: DofResolve) -> Option<Vec<Vec4>> {
    install_test_stubs();
    let (device, queue) = test_device("dof resolve pass")?;
    let pass = DofResolvePass::new(&device);
    let sharp = hdr_texture(
        &device,
        &queue,
        SIZE,
        &grid(SIZE, sharp_texel),
        wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let target = hdr_texture(
        &device,
        &queue,
        SIZE,
        &grid(SIZE, |_, _| Vec4::ZERO),
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let blurred = hdr_texture(
        &device,
        &queue,
        BLURRED_SIZE,
        &grid(BLURRED_SIZE, blurred_texel),
        wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depths: Vec<f32> = (0..SIZE.y)
        .flat_map(|y| (0..SIZE.x).map(move |x| depth_at(x, y)))
        .collect();
    let depth = float_depth_texture(&device, &queue, SIZE, &depths);
    let params = params_buffer(&device, registers);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    pass.encode(
        &device,
        &mut encoder,
        &view_of(&target),
        &view_of(&blurred),
        &SceneInputs {
            params: &params,
            sharp: &view_of(&sharp),
            merged_depth: &view_of(&depth),
        },
        variant,
    );
    queue.submit(std::iter::once(encoder.finish()));
    Some(read_hdr(&device, &queue, &target, SIZE))
}

fn expected(x: u32, y: u32) -> Vec4 {
    let blurred = blurred_texel(x * BLURRED_SIZE.x / SIZE.x, y * BLURRED_SIZE.y / SIZE.y);
    sharp_texel(x, y) + blurred + depth_texel(depth_at(x, y))
}

#[test]
fn every_pixel_reads_its_own_sharp_and_depth_texel_and_the_blurred_texel_under_it() {
    let Some(texels) = run_pass(&probe_registers(), DofResolve::Plain) else {
        return;
    };
    assert_eq!(texels, grid(SIZE, expected));
}

#[test]
fn the_fogged_variant_runs_the_fogged_entry() {
    let registers = probe_registers();
    let Some(texels) = run_pass(&registers, DofResolve::Fogged) else {
        return;
    };
    assert_eq!(texels, grid(SIZE, |x, y| expected(x, y) + registers.cb2[4]));
}
