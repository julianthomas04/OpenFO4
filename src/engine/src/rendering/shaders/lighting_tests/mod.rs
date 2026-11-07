//! GPU tests of the lighting functions: the real WGSL of the object shader runs in a compute
//! harness. Every test needs a GPU adapter and is ignored by default.

mod effects;
mod fog;
mod harness;
mod properties;
mod vectors;

use glam::Vec3;
use harness::{Case, Environment, LightingHarness, Output};

/// Relative tolerance of a sample row.
const RELATIVE_TOLERANCE: f32 = 1e-3;
/// Absolute tolerance of a sample row.
const ABSOLUTE_TOLERANCE: f32 = 1e-4;

fn gpu() -> &'static LightingHarness {
    LightingHarness::shared()
        .unwrap_or_else(|reason| panic!("the lighting tests need a GPU: {reason}"))
}

fn run(cases: &[Case]) -> Vec<Output> {
    gpu().run(&Environment::default(), cases)
}

fn run_in(environment: &Environment, cases: &[Case]) -> Vec<Output> {
    gpu().run(environment, cases)
}

fn is_close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= (RELATIVE_TOLERANCE * expected.abs()).max(ABSOLUTE_TOLERANCE)
}

fn assert_close(actual: f32, expected: f32, row: &str) {
    assert!(
        is_close(actual, expected),
        "{row}: got {actual}, expected {expected}"
    );
}

fn assert_close3(actual: Vec3, expected: Vec3, row: &str) {
    for channel in 0..3 {
        assert_close(
            actual[channel],
            expected[channel],
            &format!("{row} channel {channel}"),
        );
    }
}
