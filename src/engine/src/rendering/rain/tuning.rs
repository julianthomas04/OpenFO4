//! What the rain is tuned by: the numbers that set how it looks and what it costs. Distances are
//! written in metres. None of them is worked out from another, so each can be changed on its own.

use crate::bethesda_space::ENGINE_UNITS_PER_METER;

/// `length` metres in engine units.
const fn metres(length: f32) -> f32 {
    length * ENGINE_UNITS_PER_METER
}

/// Drops per unit of the weather's particle density at full intensity. Every drop is a card
/// the GPU fills, so this is the knob for the fill rate of a downpour.
pub const DROPS_PER_DENSITY: f32 = 1000.0;
/// The most drops a frame draws, however dense the weather asks the rain to be.
pub const MAX_DROPS: u32 = 12_000;
/// The length of a streak along its fall, in metres per unit of the weather's particle size x.
pub const STREAK_LENGTH_SCALE: f32 = 1.0;
/// The width of a streak across its fall, in metres per unit of the weather's particle size y.
pub const STREAK_WIDTH_SCALE: f32 = 1.0;
/// The rain never falls through a box smaller than this, whatever the weather asks for.
pub const MIN_BOX_EDGE: f32 = metres(4.0);
/// A drop is invisible at the first of these distances from the camera and fully visible at
/// the second.
pub const NEAR_FADE: [f32; 2] = [metres(0.6), metres(3.0)];
/// The share of half the box edge a drop is fully visible out to; beyond it the drop fades out
/// before it leaves the box on one side and enters it on the other.
pub const BOX_FADE_FROM: f32 = 0.6;
/// How opaque the alpha of the streak texture is drawn.
pub const STREAK_OPACITY: f32 = 1.0;
/// The least a drop's brightness varies to from one drop to the next.
pub const LEAST_VARIATION: f32 = 0.6;

/// The splashes at full intensity.
pub const MAX_SPLASHES: u32 = 2_000;
/// The seconds one splash plays its frames for, before it lands somewhere else.
pub const SPLASH_SECONDS: f64 = 0.5;
/// The edge of a splash's square.
pub const SPLASH_SIZE: f32 = metres(0.5);
/// How far from the camera splashes are placed.
pub const SPLASH_RADIUS: f32 = metres(25.0);
/// The same share as [`BOX_FADE_FROM`], for a splash and the circle of [`SPLASH_RADIUS`].
pub const SPLASH_FADE_FROM: f32 = 0.7;
/// How opaque the alpha of the splash atlas is drawn.
pub const SPLASH_OPACITY: f32 = 2.0;

/// The edge of the occlusion map in texels.
pub const MAP_RESOLUTION: u32 = 512;
/// Half the side of the square the occlusion map covers.
pub const HALF_EXTENT: f32 = metres(48.0);
/// Half the span of heights the occlusion map's depth covers.
pub const HALF_HEIGHT: f32 = metres(80.0);
/// The map is drawn again once the camera strays further than this share of the half extent,
/// or of the half height, from its centre.
pub const REFRESH_FRACTION: f32 = 0.25;
/// Casters spanning fewer texels than this are no roof.
pub const MIN_CASTER_TEXELS: f32 = 4.0;

/// The drops that strike a disc of water in a second at full intensity.
pub const IMPACTS_PER_SECOND: f32 = 90.0;
/// A longer frame counts as this long, so a stall does not release a burst of ripples.
pub const LONGEST_STEP_SECONDS: f32 = 0.25;
