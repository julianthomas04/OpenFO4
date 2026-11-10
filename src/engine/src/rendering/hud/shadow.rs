//! The drop shadow of HUD elements. The game's HUD movie stores no shadow (it has no filters), so
//! these values are native-engine statics this engine approximates.

use super::field_text::Shadow;
use crate::rendering::ui_colour::UiTint;

/// Offset of a quad's shadow, in stage pixels.
pub const QUAD_OFFSET_STAGE_PX: f32 = 1.5;
/// Colour of a quad's shadow: black at this alpha.
pub const QUAD_COLOUR: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// Offset of a text shadow as a fraction of the font height.
pub const TEXT_OFFSET_EM: f32 = 0.07;
/// Darkness of a text shadow, as a fraction of the opacity-scaled text alpha.
pub const TEXT_ALPHA: f32 = 0.65;

/// The shadow of a text `size` stage pixels tall, drawn at `scale` framebuffer pixels per stage
/// pixel.
pub fn text(size: f32, scale: f32, tint: &UiTint) -> Shadow {
    Shadow {
        offset: size * scale * TEXT_OFFSET_EM,
        colour: [0, 0, 0, tint.shadow_alpha(TEXT_ALPHA)],
    }
}
