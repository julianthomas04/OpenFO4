//! How far each cloud layer's texture has scrolled. The offsets are visual state in real seconds:
//! they do not follow the world clock, so pausing or speeding up time does not stop the clouds.

use glam::Vec2;

use super::clouds::CloudLayerDraw;
use super::layer::finite_or_zero;
use crate::rendering::lighting::time_of_day::unmeasured::CLOUD_SCROLL_SCALE;

/// Every value of a layer index of the weather record.
const LAYER_INDEX_COUNT: usize = u8::MAX as usize + 1;

/// `offset` moved by `velocity` texture widths per second over `dt` seconds, wrapped into
/// 0 to 1 on both axes (the textures repeat). A time step that is negative or not a number
/// moves nothing.
pub(super) fn wrap_scroll(offset: Vec2, velocity: Vec2, dt: f32) -> Vec2 {
    let dt = finite_or_zero(dt).max(0.0);
    let moved = offset.map(finite_or_zero) + velocity.map(finite_or_zero) * dt;
    moved.map(wrap_unit)
}

fn wrap_unit(value: f32) -> f32 {
    let wrapped = value.rem_euclid(1.0);
    // A tiny negative value rounds up to exactly 1.
    if wrapped < 1.0 { wrapped } else { 0.0 }
}

/// The scroll offset of every cloud layer by layer index. A layer index that two weathers show
/// at once during a transition has one offset, so the layer does not jump when one of them goes.
pub(super) struct ScrollOffsets([Vec2; LAYER_INDEX_COUNT]);

impl ScrollOffsets {
    pub(super) fn new() -> Self {
        Self([Vec2::ZERO; LAYER_INDEX_COUNT])
    }

    pub(super) fn offset(&self, layer: u8) -> Vec2 {
        self.0[usize::from(layer)]
    }

    /// Moves every layer in `layers` on by `dt` seconds. A layer that is there more than once
    /// moves once, at the mean of its speeds weighted by their alphas.
    pub(super) fn advance(&mut self, layers: &[CloudLayerDraw], dt: f32) {
        for (position, layer) in layers.iter().enumerate() {
            if layers[..position]
                .iter()
                .any(|earlier| earlier.layer == layer.layer)
            {
                continue;
            }
            let same_layer = layers.iter().filter(|other| other.layer == layer.layer);
            let velocity = mean_speed(same_layer) * CLOUD_SCROLL_SCALE;
            let offset = &mut self.0[usize::from(layer.layer)];
            *offset = wrap_scroll(*offset, velocity, dt);
        }
    }
}

/// The mean scroll speed of `layers`, weighted by their alphas; a plain mean while none shows.
fn mean_speed<'a>(layers: impl Iterator<Item = &'a CloudLayerDraw> + Clone) -> Vec2 {
    let speed = |layer: &CloudLayerDraw| layer.scroll_speed.map(finite_or_zero);
    let weight = |layer: &CloudLayerDraw| finite_or_zero(layer.alpha).clamp(0.0, 1.0);
    let total_weight: f32 = layers.clone().map(weight).sum();
    if total_weight > 0.0 {
        layers
            .map(|layer| speed(layer) * weight(layer))
            .sum::<Vec2>()
            / total_weight
    } else {
        let count = layers.clone().count();
        layers.map(speed).sum::<Vec2>() / count as f32
    }
}
