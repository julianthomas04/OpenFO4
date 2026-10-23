//! The drop shadow of HUD elements. The game's HUD movie stores no shadow (it has no filters), so
//! these values are native-engine statics this engine approximates.

/// Offset of a quad's shadow, in stage pixels.
pub const QUAD_OFFSET_STAGE_PX: f32 = 1.5;
/// Colour of a quad's shadow: black at this alpha.
pub const QUAD_COLOUR: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// Offset of a text shadow as a fraction of the font height.
pub const TEXT_OFFSET_EM: f32 = 0.07;
/// Darkness of a text shadow, as a fraction of the opacity-scaled text alpha.
pub const TEXT_ALPHA: f32 = 0.65;
