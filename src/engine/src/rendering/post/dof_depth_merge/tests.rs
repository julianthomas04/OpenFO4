#![allow(
    clippy::float_cmp,
    reason = "the merged depth is one of the inputs or the sky clamp, bit for bit"
)]

//! The depth merge pass on the GPU: the nearer of scene and surface depth, reverse-Z.

use glam::UVec2;

use super::*;
use crate::rendering::{
    gpu_test::test_device,
    post::dof_test_support::{
        float_depth_texture, install_test_stubs, read_depth, scene_depth_view, view_of,
    },
};

const SIZE: UVec2 = UVec2::new(4, 2);
const SKY_CLAMP: f32 = 1e-7;

fn merge(scene: &[f32], surface: &[f32]) -> Option<Vec<f32>> {
    install_test_stubs();
    let (device, queue) = test_device("dof depth merge")?;
    let merge = DofDepthMerge::new(&device, SIZE.x, SIZE.y);
    let scene_view = scene_depth_view(&device, &queue, SIZE, scene);
    let surface_view = view_of(&float_depth_texture(&device, &queue, SIZE, surface));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    merge.encode(&device, &mut encoder, &scene_view, &surface_view);
    queue.submit(std::iter::once(encoder.finish()));
    Some(read_depth(&device, &queue, merge.texture(), SIZE))
}

#[test]
fn the_nearer_of_scene_and_surface_depth_wins_per_pixel() {
    let scene = [0.5, 0.25, 0.125, 0.0625, 0.5, 0.25, 0.125, 0.0625];
    let surface = [0.25, 0.5, 0.0, 0.125, 0.5, 0.0, 0.25, 0.0625];
    let Some(merged) = merge(&scene, &surface) else {
        return;
    };
    assert_eq!(merged, [0.5, 0.5, 0.125, 0.125, 0.5, 0.25, 0.25, 0.0625]);
}

#[test]
fn a_zero_surface_depth_leaves_the_scene_depth() {
    let scene = [0.5, 0.25, 0.125, 0.0625, 0.5, 0.25, 0.125, 0.0625];
    let Some(merged) = merge(&scene, &[0.0; 8]) else {
        return;
    };
    assert_eq!(merged, scene);
}

#[test]
fn the_sky_depth_is_clamped_to_a_finite_horizon() {
    let Some(merged) = merge(&[0.0; 8], &[0.0; 8]) else {
        return;
    };
    assert!(merged.iter().all(|&depth| depth == SKY_CLAMP));
}

#[test]
fn the_merged_target_follows_the_scene_size() {
    install_test_stubs();
    let Some((device, _)) = test_device("dof depth merge size") else {
        return;
    };
    let mut merge = DofDepthMerge::new(&device, SIZE.x, SIZE.y);
    assert_eq!(merge.texture().size().width, SIZE.x);
    merge.resize(&device, 8, 6);
    let size = merge.texture().size();
    assert_eq!((size.width, size.height), (8, 6));
}
