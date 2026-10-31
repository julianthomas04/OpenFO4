//! The sky gradient of `sky_gradient.wgsl`, run on the GPU, against the shares of `SkyUpper`,
//! `SkyLower` and Horizon the probe measured in the vanilla sky.

use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::rendering::gpu_test::{map_read, test_device};

const ENTRY_SHADER: &str = "
    @group(0) @binding(0) var<storage, read> sines: array<f32>;
    @group(0) @binding(1) var<storage, read_write> weights: array<vec4<f32>>;

    @compute @workgroup_size(1)
    fn main(@builtin(global_invocation_id) id: vec3<u32>) {
        weights[id.x] = vec4<f32>(sky_gradient_weights(sines[id.x]), 0.0);
    }";

/// Elevation in degrees, then the measured `SkyUpper`, `SkyLower` and Horizon shares: the medians of
/// the noon sky in 1 degree bins over the north, east, south, south-high and zenith poses
/// (`results_v10`, "How do `SkyUpper`, `SkyLower` and Horizon spread over the sky?").
const MEASURED: [(f32, f32, f32, f32); 25] = [
    (1.5, 0.000, 0.368, 0.632),
    (2.5, 0.000, 0.496, 0.504),
    (3.5, 0.000, 0.595, 0.405),
    (4.5, 0.000, 0.669, 0.331),
    (5.5, 0.000, 0.743, 0.257),
    (6.5, 0.000, 0.801, 0.199),
    (7.5, 0.041, 0.812, 0.157),
    (8.5, 0.069, 0.811, 0.120),
    (9.5, 0.123, 0.792, 0.070),
    (10.5, 0.195, 0.763, 0.042),
    (11.5, 0.269, 0.703, 0.029),
    (12.5, 0.341, 0.635, 0.028),
    (13.5, 0.410, 0.590, 0.000),
    (15.5, 0.515, 0.485, 0.000),
    (17.5, 0.595, 0.405, 0.000),
    (20.5, 0.680, 0.320, 0.000),
    (25.5, 0.782, 0.218, 0.000),
    (30.5, 0.838, 0.162, 0.000),
    (35.5, 0.877, 0.123, 0.000),
    (40.5, 0.909, 0.091, 0.000),
    (45.5, 0.930, 0.070, 0.000),
    (55.5, 0.957, 0.043, 0.000),
    (65.5, 0.971, 0.029, 0.000),
    (75.5, 1.000, 0.000, 0.000),
    (85.5, 1.000, 0.000, 0.000),
];

/// The largest gap the fit leaves to a measured share (the lowest bin, where the horizon is
/// steepest), and the largest rms over these rows of one colour. Over every 1 degree bin the fit
/// leaves 0.014, 0.018 and 0.010; these rows weigh the low sky, where it fits worst.
const MAX_GAP: f32 = 0.07;
const MAX_RMS: f32 = 0.03;
/// How far the horizon row may miss pure Horizon: `pow(1.0, y)` is not exact on every adapter
/// (CI's software adapter gives 0.9991).
const HORIZON_ROW_TOLERANCE: f32 = 2e-3;

fn weights_at(test: &str, sines: &[f32]) -> Option<Vec<Vec3>> {
    let (device, queue) = test_device(test)?;
    let source = format!(
        "{}{ENTRY_SHADER}",
        include_str!("../shaders/sky_gradient.wgsl")
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky_gradient_test"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("sky_gradient_test"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("sky_gradient_test_sines"),
        contents: bytemuck::cast_slice(sines),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (sines.len() * std::mem::size_of::<[f32; 4]>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sky_gradient_test_weights"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sky_gradient_test_staging"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(sines.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, size);
    queue.submit(std::iter::once(encoder.finish()));
    let raw: Vec<[f32; 4]> = bytemuck::cast_slice(&map_read(&device, &staging)).to_vec();
    Some(
        raw.into_iter()
            .map(|[x, y, z, _]| Vec3::new(x, y, z))
            .collect(),
    )
}

fn sine_of(elevation_deg: f32) -> f32 {
    elevation_deg.to_radians().sin()
}

#[test]
fn the_gradient_follows_the_measured_shares_of_the_three_sky_colours() {
    let sines: Vec<f32> = MEASURED.iter().map(|row| sine_of(row.0)).collect();
    let Some(weights) = weights_at("sky_gradient_measured_shares", &sines) else {
        return;
    };
    let mut squared = Vec3::ZERO;
    for (&(elevation, upper, lower, horizon), weight) in MEASURED.iter().zip(&weights) {
        let gap = (*weight - Vec3::new(upper, lower, horizon)).abs();
        assert!(
            gap.max_element() <= MAX_GAP,
            "{elevation} degrees: {weight} against ({upper}, {lower}, {horizon})"
        );
        squared += gap * gap;
    }
    let rms = (squared / MEASURED.len() as f32).powf(0.5);
    assert!(rms.max_element() <= MAX_RMS, "rms {rms}");
}

#[test]
fn the_shares_add_up_to_one_and_the_horizon_row_is_pure_horizon() {
    let sines: Vec<f32> = (0..=40).map(|step| step as f32 / 40.0).collect();
    let Some(weights) = weights_at("sky_gradient_sums", &sines) else {
        return;
    };
    for (sine, weight) in sines.iter().zip(&weights) {
        assert!(weight.min_element() >= 0.0, "{sine}: {weight}");
        assert!(
            (weight.element_sum() - 1.0).abs() < 1e-5,
            "{sine}: {weight}"
        );
    }
    assert!(
        weights[0].abs_diff_eq(Vec3::Z, HORIZON_ROW_TOLERANCE),
        "{}",
        weights[0]
    );
}

#[test]
fn horizon_gives_way_to_sky_upper_as_the_elevation_rises() {
    let sines: Vec<f32> = (0..=90).map(|degrees| sine_of(degrees as f32)).collect();
    let Some(weights) = weights_at("sky_gradient_monotone", &sines) else {
        return;
    };
    for pair in weights.windows(2) {
        assert!(pair[1].x >= pair[0].x - 1e-6, "{pair:?}");
        assert!(pair[1].z <= pair[0].z + 1e-6, "{pair:?}");
    }
}
