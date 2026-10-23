//! Tests of the SSR chain: the scheduling policy, pipeline validation against the
//! wrapper layouts, and the plumbing of the targets and passes. The GPU tests skip
//! without an adapter (see `gpu_test`) and run the wrappers against the synthetic
//! translated stand-ins of `composed::ssr::stubs`.
#![allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact copies"
)]

pub(crate) mod harness;

use glam::{UVec2, Vec3};
use wgpu::util::DeviceExt;

use self::harness::*;
use super::{
    constants::half_extent,
    pipelines::SsrPipelines,
    targets::{HIZ_MIP_COUNT, hiz_level_extent},
    *,
};
use crate::rendering::{camera::Camera, gpu_test::test_device, scene_copy::SceneCopy};

fn surface(wading: bool, lod: bool) -> SurfaceSsr {
    SurfaceSsr {
        lod,
        wading,
        water_flag: true,
        camera_underwater: false,
        shading_samples_ssr: true,
    }
}

#[test]
fn a_surface_needs_the_watr_flag_a_dry_camera_and_a_shading_pipeline_with_taps() {
    let wanting = surface(false, false);
    assert_eq!(ray_setup_kind(&wanting), Some(RaySetupKind::Surface));
    for denied in [
        SurfaceSsr {
            water_flag: false,
            ..wanting
        },
        SurfaceSsr {
            camera_underwater: true,
            ..wanting
        },
        SurfaceSsr {
            shading_samples_ssr: false,
            ..wanting
        },
    ] {
        assert_eq!(ray_setup_kind(&denied), None, "{denied:?}");
    }
}

#[test]
fn lod_and_wading_surfaces_pick_their_own_ray_setup() {
    assert_eq!(
        ray_setup_kind(&surface(false, true)),
        Some(RaySetupKind::Lod)
    );
    assert_eq!(
        ray_setup_kind(&surface(true, false)),
        Some(RaySetupKind::Wading)
    );
}

#[test]
fn wading_ray_setups_draw_after_the_regular_ones() {
    assert_eq!(
        RaySetupKind::ALL,
        [
            RaySetupKind::Surface,
            RaySetupKind::Lod,
            RaySetupKind::Wading
        ]
    );
}

#[test]
fn every_ssr_pipeline_validates_against_the_shader_layouts() {
    let Some((device, _queue)) = test_device("ssr pipeline validation") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = SceneCopy::new(&device, 4, 4);
    let layouts = crate::rendering::water::pipelines::WaterLayouts::new(&device);
    let pipelines = SsrPipelines::new(&device, scene.layout(), &layouts);
    for kind in RaySetupKind::ALL {
        let _ = pipelines.ray_setup(kind);
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

const SMOKE_SIZE: UVec2 = UVec2::new(64, 64);

fn ssr_test_camera() -> SsrCamera {
    SsrCamera::of(&Camera {
        eye: Vec3::ZERO,
        target: Vec3::NEG_Z,
        up: Vec3::Y,
        aspect: 1.0,
        fovy: 90.0,
        znear: 0.5,
        zfar: 100_000.0,
    })
}

#[test]
fn the_chain_encodes_every_pass_and_fills_every_hiz_mip() {
    let Some((device, queue)) = test_device("ssr chain plumbing") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layouts = crate::rendering::water::pipelines::WaterLayouts::new(&device);
    let scene_copy = SceneCopy::new(&device, SMOKE_SIZE.x, SMOKE_SIZE.y);
    let mut chain = SsrChain::new(&device, scene_copy.layout(), &layouts);
    chain.prepare(&device, &queue, SMOKE_SIZE, &ssr_test_camera());
    let targets = chain.targets().expect("prepare allocated the targets");
    let half = targets.half();
    assert_eq!(half, half_extent(SMOKE_SIZE.x, SMOKE_SIZE.y));

    let texels = (SMOKE_SIZE.x * SMOKE_SIZE.y) as usize;
    let hdr = device.create_texture_with_data(
        &queue,
        &wgpu::TextureDescriptor {
            label: Some("ssr_test_scene_colour"),
            size: extent(SMOKE_SIZE),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::rendering::post::hdr_target::HDR_FORMAT,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &vec![0_u8; texels * std::mem::size_of::<[u16; 4]>()],
    );
    let depth = depth_texture(&device, &queue, SMOKE_SIZE, &vec![0.5; texels]);

    let profiler = GpuProfiler::new(&device, &queue);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    scene_copy.encode(&mut encoder, &hdr, &depth);
    chain.encode_resolve(
        &device,
        &mut encoder,
        &scene_copy,
        &depth.create_view(&wgpu::TextureViewDescriptor::default()),
        &profiler,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let hiz = &targets.textures.hiz;
    assert_eq!(hiz.mip_level_count(), HIZ_MIP_COUNT);
    for level in 0..HIZ_MIP_COUNT {
        let extent = hiz_level_extent(half, level);
        let texels = read_f32_mip(&device, &queue, hiz, level, extent);
        assert_eq!(texels.len(), (extent.x * extent.y) as usize, "mip {level}");
        assert!(
            texels.iter().any(|texel| *texel > 0.0),
            "mip {level} was written by the Hi-Z passes"
        );
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

#[test]
fn preparing_again_with_the_same_size_keeps_the_targets() {
    let Some((device, queue)) = test_device("ssr target reuse") else {
        return;
    };
    let scene_copy = SceneCopy::new(&device, 64, 64);
    let layouts = crate::rendering::water::pipelines::WaterLayouts::new(&device);
    let mut chain = SsrChain::new(&device, scene_copy.layout(), &layouts);
    let camera = ssr_test_camera();
    let first = chain.prepare(&device, &queue, UVec2::new(64, 64), &camera);
    let second = chain.prepare(&device, &queue, UVec2::new(64, 64), &camera);
    assert_eq!(first.sharp, second.sharp);
    let resized = chain.prepare(&device, &queue, UVec2::new(128, 64), &camera);
    assert_ne!(
        first.sharp, resized.sharp,
        "a new size recreates the targets"
    );
    assert_eq!(
        chain.targets().map(SsrTargets::half),
        Some(half_extent(128, 64))
    );
}
