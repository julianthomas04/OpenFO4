//! Test support: the one headless GPU device every GPU test shares; the translated shaders
//! are the synthetic stubs.
//!
//! A device per test means an instance per test, and tearing instances down while others are
//! still being created hangs the driver on Windows. `Device` and `Queue` are `Send + Sync`
//! handles and error scopes are per thread, so tests share the device without seeing each
//! other's errors.
//!
//! The device also carries whichever production features the adapter offers, so shaders the
//! game compiles with those features (the binding arrays of the object shader) compile in a
//! test too.

use std::sync::OnceLock;

use crate::rendering::{
    device::{REQUIRED_FEATURES, STORAGE_BUFFERS_PER_STAGE},
    instance,
    water::displacement::REQUIRED_FORMAT_FEATURES,
};

const REQUIRE_GPU_ENV: &str = "OPENFO4_REQUIRE_GPU_TESTS";

static SHARED_DEVICE: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();

/// The shared device and queue, or `None` after logging which test was skipped. Setting
/// `OPENFO4_REQUIRE_GPU_TESTS=1` turns the skip into a failure (for CI runners that are
/// meant to have an adapter).
pub fn test_device(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    crate::rendering::shaders::startup::install_test_stubs();
    match SHARED_DEVICE.get_or_init(create_device) {
        Ok((device, queue)) => Some((device.clone(), queue.clone())),
        Err(reason) => {
            assert!(
                std::env::var_os(REQUIRE_GPU_ENV).is_none(),
                "{REQUIRE_GPU_ENV} is set but the GPU test `{test}` cannot run: {reason}"
            );
            eprintln!("SKIPPED GPU test `{test}`: {reason}");
            None
        }
    }
}

fn create_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let adapter = pollster::block_on(instance::new().request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        },
    ))
    .map_err(|error| format!("no adapter: {error}"))?;
    let mut limits = wgpu::Limits::default();
    limits.max_storage_buffers_per_shader_stage =
        STORAGE_BUFFERS_PER_STAGE.min(adapter.limits().max_storage_buffers_per_shader_stage);
    let descriptor = wgpu::DeviceDescriptor {
        required_features: REQUIRED_FORMAT_FEATURES | (adapter.features() & REQUIRED_FEATURES),
        required_limits: limits,
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&descriptor))
        .map_err(|error| format!("no device: {error}"))
}
