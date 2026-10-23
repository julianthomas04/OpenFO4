#![allow(
    clippy::float_cmp,
    reason = "every value is a binary fraction, exact through the half-float targets"
)]

//! The whole effect on the GPU against the synthetic pass stand-ins: the scene copy, the
//! depth merge and the pass chain feed each other as intended.

use glam::{UVec2, Vec4};

use super::*;
use crate::rendering::{
    gpu_test::test_device,
    post::dof_test_support::{
        depth_texel, float_depth_texture, hdr_texture, install_test_stubs, probe_registers,
        read_hdr, scene_depth_view, view_of,
    },
};

const SIZE: UVec2 = UVec2::splat(4);
const SCENE_COLOUR: Vec4 = Vec4::new(0.5, 0.25, 0.75, 1.0);
const SCENE_DEPTH: f32 = 0.125;

fn run(resolve: DofResolve, surface_depth: Option<f32>) -> Option<Vec<Vec4>> {
    install_test_stubs();
    let (device, queue) = test_device("bokeh dof")?;
    let mut effect = BokehDepthOfField::new(&device, 2, 2);
    effect.resize(&device, SIZE.x, SIZE.y);
    let texel_count = (SIZE.x * SIZE.y) as usize;
    let hdr = hdr_texture(
        &device,
        &queue,
        SIZE,
        &vec![SCENE_COLOUR; texel_count],
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    let depth = scene_depth_view(&device, &queue, SIZE, &vec![SCENE_DEPTH; texel_count]);
    let surface = surface_depth.map(|depth| {
        view_of(&float_depth_texture(
            &device,
            &queue,
            SIZE,
            &vec![depth; texel_count],
        ))
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    effect.encode(
        &device,
        &queue,
        &mut encoder,
        &hdr,
        &view_of(&hdr),
        &depth,
        surface.as_ref(),
        &probe_registers(),
        resolve,
    );
    queue.submit(std::iter::once(encoder.finish()));
    Some(read_hdr(&device, &queue, &hdr, SIZE))
}

/// What the stand-in chain makes of a uniform scene: the circle of confusion pass writes
/// `cb2[3] + scene + depth`, each blur pass adds `cb2[1]`, the resolve adds scene and depth.
fn expected(resolve: DofResolve, depth: f32) -> Vec4 {
    let [_, blur_step, _, depth_registers, fog, _] = probe_registers().cb2;
    let depth = depth_texel(depth);
    let blurred = depth_registers + SCENE_COLOUR + depth + blur_step + blur_step;
    let fogged = match resolve {
        DofResolve::Plain => Vec4::ZERO,
        DofResolve::Fogged => fog,
    };
    blurred + SCENE_COLOUR + depth + fogged
}

#[test]
fn the_plain_chain_reads_the_scene_copy_and_the_scene_depth() {
    let Some(texels) = run(DofResolve::Plain, None) else {
        return;
    };
    assert!(
        texels
            .iter()
            .all(|&texel| texel == expected(DofResolve::Plain, SCENE_DEPTH))
    );
}

#[test]
fn the_fogged_chain_runs_the_fogged_resolve() {
    let Some(texels) = run(DofResolve::Fogged, None) else {
        return;
    };
    assert!(
        texels
            .iter()
            .all(|&texel| texel == expected(DofResolve::Fogged, SCENE_DEPTH))
    );
}

#[test]
fn a_nearer_underwater_surface_decides_the_depth_the_effect_sees() {
    let Some(texels) = run(DofResolve::Plain, Some(0.5)) else {
        return;
    };
    assert!(
        texels
            .iter()
            .all(|&texel| texel == expected(DofResolve::Plain, 0.5))
    );
}

#[test]
fn a_farther_underwater_surface_leaves_the_scene_depth() {
    let Some(texels) = run(DofResolve::Plain, Some(0.0625)) else {
        return;
    };
    assert!(
        texels
            .iter()
            .all(|&texel| texel == expected(DofResolve::Plain, SCENE_DEPTH))
    );
}

#[test]
fn no_surface_depth_equals_a_cleared_surface_texture() {
    let (Some(without), Some(cleared)) = (
        run(DofResolve::Plain, None),
        run(DofResolve::Plain, Some(0.0)),
    ) else {
        return;
    };
    assert_eq!(without, cleared);
}
