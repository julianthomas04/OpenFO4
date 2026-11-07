//! The sky gradient pipeline, drawn with the night sky's test rig: which frame colour stands for
//! which weather colour.

use glam::Vec4;

use super::tests::gpu::{SKY_CLEAR, frame_uniform, render, rig_with_frame, texel};
use crate::rendering::post::sky::SkyPipeline;

const COLUMN: u32 = 8;
/// About 43, 10.6 and 3.6 degrees above the horizon in the 90 degree view of the rig.
const HIGH_ROW: u32 = 0;
const LOW_BAND_ROW: u32 = 6;
const NEAR_HORIZON_ROW: u32 = 7;

#[test]
fn sky_upper_lower_and_horizon_reach_the_zenith_ground_and_horizon_colours_of_the_frame() {
    let mut frame = frame_uniform();
    frame.inv_view_proj = frame.view_proj.inverse();
    frame.sky_zenith = Vec4::new(1.0, 0.0, 0.0, 0.0);
    frame.sky_ground = Vec4::new(0.0, 1.0, 0.0, 0.0);
    frame.sky_horizon = Vec4::new(0.0, 0.0, 1.0, 0.0);
    let Some(rig) = rig_with_frame("sky_gradient_frame_colours", &frame) else {
        return;
    };
    let layout = rig
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_gradient_test_layout"),
            bind_group_layouts: &rig.world_groups().map(Some),
            immediate_size: 0,
        });
    let sky = SkyPipeline::new(&rig.device, &layout);

    let pixels = render(&rig, SKY_CLEAR, |pass| sky.draw(pass));

    for row in [HIGH_ROW, LOW_BAND_ROW, NEAR_HORIZON_ROW] {
        let shown = texel(&pixels, COLUMN, row);
        assert!(
            (shown.x + shown.y + shown.z - 1.0).abs() < 1e-3,
            "row {row}: {shown}"
        );
    }
    let high = texel(&pixels, COLUMN, HIGH_ROW);
    assert!(high.x > 0.85, "SkyUpper rules high up: {high}");
    let band = texel(&pixels, COLUMN, LOW_BAND_ROW);
    assert!(
        band.y > band.x && band.y > band.z,
        "SkyLower rules the low band: {band}"
    );
    let near = texel(&pixels, COLUMN, NEAR_HORIZON_ROW);
    assert_eq!(near.x, 0.0, "no SkyUpper near the horizon: {near}");
    assert!(near.z > 0.3, "Horizon near the horizon: {near}");
}
