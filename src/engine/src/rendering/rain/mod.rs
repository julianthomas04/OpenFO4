//! Rain: the top-down occlusion map that tells which spots lie under a roof, and the passes that
//! draw the rain from it. Everything here takes plain data; the frame decides when it runs.

pub mod occlusion;

use crate::bethesda_space::UNITS_TO_ENGINE;

/// Bethesda game units to the metre.
const GAME_UNITS_PER_METRE: f32 = 70.0;

/// Engine units to the metre: the scale the rain constants are written in.
pub const ENGINE_UNITS_PER_METRE: f32 = GAME_UNITS_PER_METRE * UNITS_TO_ENGINE;
