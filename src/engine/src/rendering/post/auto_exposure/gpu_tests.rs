//! The auto exposure pass on the GPU: which luminance statistic it meters, how the image space's
//! limits bound the exposure, and how the adapted luminance follows a brighter or darker picture.

use glam::{UVec2, Vec4};

use super::{AutoExposure, AutoExposureParams, STATE_BYTES};
use crate::rendering::{
    gpu_profiler::GpuProfiler,
    gpu_test::{map_read, test_device},
    lighting::{state::ExposureReadout, time_of_day::unmeasured::SUN_DISC_RADIANCE},
    post::dof_test_support::{hdr_texture, view_of},
};

/// One texel per sample of the metering grid.
const SIZE: UVec2 = UVec2::new(64, 64);
const TARGET: f32 = 0.09;

fn params(min: f32, max: f32) -> AutoExposureParams {
    AutoExposureParams {
        adaptation: 1.0,
        darkening: 0.0,
        target_luminance: TARGET,
        min,
        max,
        ev_scale: 1.0,
        auto: 1,
    }
}

fn unclamped() -> AutoExposureParams {
    params(0.01, 100.0)
}

struct Meter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    exposure: AutoExposure,
    profiler: GpuProfiler,
}

impl Meter {
    fn new(test: &str) -> Option<Self> {
        let (device, queue) = test_device(test)?;
        let scene = Self::scene(&device, &queue, |_, _| 1.0);
        let exposure = AutoExposure::new(&device, &view_of(&scene));
        let profiler = GpuProfiler::new(&device, &queue);
        Some(Self {
            device,
            queue,
            exposure,
            profiler,
        })
    }

    /// A grey picture whose texel at (x, y) has `luminance(x, y)`.
    fn scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        luminance: impl Fn(u32, u32) -> f32,
    ) -> wgpu::Texture {
        let texels: Vec<Vec4> = (0..SIZE.y)
            .flat_map(|y| (0..SIZE.x).map(move |x| (x, y)))
            .map(|(x, y)| Vec4::new(luminance(x, y), luminance(x, y), luminance(x, y), 1.0))
            .collect();
        hdr_texture(
            device,
            queue,
            SIZE,
            &texels,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    /// The state after metering one frame of a picture whose left half has `left` and right
    /// half `right` as grey luminance.
    fn frame(&mut self, left: f32, right: f32, params: &AutoExposureParams) -> ExposureReadout {
        self.picture(|x, _| if x < SIZE.x / 2 { left } else { right }, params)
    }

    fn picture(
        &mut self,
        luminance: impl Fn(u32, u32) -> f32,
        params: &AutoExposureParams,
    ) -> ExposureReadout {
        let scene = Self::scene(&self.device, &self.queue, luminance);
        self.exposure.rebind_source(&self.device, &view_of(&scene));
        self.exposure.write_params(&self.queue, params);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto_exposure_test_readback"),
            size: STATE_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.exposure.encode(&mut encoder, &self.profiler);
        encoder.copy_buffer_to_buffer(&self.exposure.state, 0, &readback, 0, STATE_BYTES);
        self.queue.submit(std::iter::once(encoder.finish()));
        *bytemuck::from_bytes(&map_read(&self.device, &readback))
    }
}

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual / expected - 1.0).abs() < 1e-3,
        "{what}: {actual}, expected {expected}"
    );
}

#[test]
fn the_meter_reads_the_mean_luminance_of_the_picture() {
    let Some(mut meter) = Meter::new("auto exposure mean") else {
        return;
    };
    let state = meter.frame(0.02, 1.0, &unclamped());
    assert_near(state.adapted_luminance, 0.51, "adapted luminance");
    assert_near(state.exposure, TARGET / 0.51, "exposure");
}

#[test]
fn the_exposure_stays_within_the_image_space_limits() {
    let Some(mut meter) = Meter::new("auto exposure limits") else {
        return;
    };
    assert_near(
        meter.frame(1.0, 1.0, &params(0.5, 6.0)).exposure,
        0.5,
        "bright picture",
    );
    assert_near(
        meter.frame(0.001, 0.001, &params(0.5, 6.0)).exposure,
        6.0,
        "dark picture",
    );
}

#[test]
fn towards_a_brighter_picture_the_adapted_luminance_blends_by_the_adaptation() {
    let Some(mut meter) = Meter::new("auto exposure brightening") else {
        return;
    };
    meter.frame(0.1, 0.1, &unclamped());
    let blended = meter.frame(
        0.5,
        0.5,
        &AutoExposureParams {
            adaptation: 0.25,
            darkening: 0.5,
            ..unclamped()
        },
    );
    assert_near(blended.adapted_luminance, 0.2, "adapted luminance");
}

#[test]
fn towards_a_darker_picture_the_adapted_luminance_falls_by_the_darkening_share() {
    let Some(mut meter) = Meter::new("auto exposure darkening") else {
        return;
    };
    meter.frame(0.5, 0.5, &unclamped());
    let darkening = AutoExposureParams {
        adaptation: 0.0,
        darkening: 0.5,
        ..unclamped()
    };
    assert_near(
        meter.frame(0.1, 0.1, &darkening).adapted_luminance,
        0.25,
        "one frame",
    );
    assert_near(
        meter.frame(0.1, 0.1, &darkening).adapted_luminance,
        0.125,
        "two frames",
    );
    assert_near(
        meter.frame(0.1, 0.1, &darkening).adapted_luminance,
        0.1,
        "never below the metered",
    );
}

/// The brightest texel the engine draws: a white sun disc at the largest sky scale of a vanilla
/// weather image space (5). The disc radiance per sky scale is not measured.
const HOT_TEXEL: f32 = SUN_DISC_RADIANCE * 5.0;

#[test]
fn one_hot_texel_counts_fully_in_the_mean() {
    let Some(mut meter) = Meter::new("auto exposure hot texel") else {
        return;
    };
    let ground = 0.1;
    let state = meter.picture(
        |x, y| {
            if (x, y) == (SIZE.x / 2, SIZE.y / 2) {
                HOT_TEXEL
            } else {
                ground
            }
        },
        &unclamped(),
    );
    let samples = (SIZE.x * SIZE.y) as f32;
    let mean = ground + (HOT_TEXEL - ground) / samples;
    assert_near(state.adapted_luminance, mean, "adapted luminance");
    assert_near(state.exposure, TARGET / mean, "exposure");
    assert!(
        mean / ground > 1.15,
        "one of 4096 samples moves the mean by {}",
        mean / ground
    );
}
