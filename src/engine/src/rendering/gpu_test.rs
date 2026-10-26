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
    cube_array::CUBE_SLOTS,
    device::{REQUIRED_FEATURES, STORAGE_BUFFERS_PER_STAGE},
    gpu_driven::MAX_TEXTURES,
    instance,
    water::displacement::REQUIRED_FORMAT_FEATURES,
};

const REQUIRE_GPU_ENV: &str = "OPENFO4_REQUIRE_GPU_TESTS";

/// The 2D and cube textures of the world pass material group.
const BINDLESS_TEXTURES: u32 = MAX_TEXTURES + CUBE_SLOTS;

static SHARED_DEVICE: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();

/// The shared device and queue, or `None` after logging which test was skipped. Setting
/// `OPENFO4_REQUIRE_GPU_TESTS=1` turns the skip into a failure (for CI runners that are
/// meant to have an adapter).
pub fn test_device(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    crate::rendering::shaders::startup::install_test_stubs();
    match SHARED_DEVICE.get_or_init(create_device) {
        Ok((device, queue)) => Some((device.clone(), queue.clone())),
        Err(reason) => skipped(test, reason),
    }
}

/// The shared device for the tests that build the world pass layouts, whose material group is
/// the bindless texture array: `None` after logging the skip when the adapter lacks a feature
/// or the room that group needs.
pub fn world_pass_device(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let (device, queue) = test_device(test)?;
    match world_pass_shortfall(&device) {
        Some(reason) => skipped(test, &reason),
        None => Some((device, queue)),
    }
}

fn skipped<T>(test: &str, reason: &str) -> Option<T> {
    assert!(
        std::env::var_os(REQUIRE_GPU_ENV).is_none(),
        "{REQUIRE_GPU_ENV} is set but the GPU test `{test}` cannot run: {reason}"
    );
    eprintln!("SKIPPED GPU test `{test}`: {reason}");
    None
}

fn world_pass_shortfall(device: &wgpu::Device) -> Option<String> {
    let missing = REQUIRED_FEATURES - device.features();
    if !missing.is_empty() {
        return Some(format!("the adapter lacks the device features {missing:?}"));
    }
    let limits = device.limits();
    (limits.max_sampled_textures_per_shader_stage < BINDLESS_TEXTURES
        || limits.max_binding_array_elements_per_shader_stage < BINDLESS_TEXTURES)
        .then(|| format!("the adapter offers fewer than {BINDLESS_TEXTURES} bindless textures"))
}

fn create_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let adapter = pollster::block_on(instance::new().request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        },
    ))
    .map_err(|error| format!("no adapter: {error}"))?;
    let adapter_limits = adapter.limits();
    let limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: STORAGE_BUFFERS_PER_STAGE
            .min(adapter_limits.max_storage_buffers_per_shader_stage),
        max_sampled_textures_per_shader_stage: BINDLESS_TEXTURES
            .min(adapter_limits.max_sampled_textures_per_shader_stage),
        max_binding_array_elements_per_shader_stage: BINDLESS_TEXTURES
            .min(adapter_limits.max_binding_array_elements_per_shader_stage),
        ..Default::default()
    };
    let descriptor = wgpu::DeviceDescriptor {
        required_features: REQUIRED_FORMAT_FEATURES | (adapter.features() & REQUIRED_FEATURES),
        required_limits: limits,
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&descriptor))
        .map_err(|error| format!("no device: {error}"))
}
