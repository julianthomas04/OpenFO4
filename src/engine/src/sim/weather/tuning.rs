//! The numbers behind the weather cycle. The game data fixes only part of the picture: the chances
//! of a climate, its volatility byte and each weather's transition and thunder bytes. How those
//! bytes become hours and seconds is our own heuristic. It lives here so it can be retuned against
//! observations of the original.

/// Game hours between two rolls in a climate without volatility, while a weather with a
/// volatility multiplier of 1 is current.
pub(super) const BASE_ROLL_INTERVAL_HOURS: f32 = 8.0;
/// The share of the base interval that is left in a climate with the highest volatility byte.
pub(super) const MOST_VOLATILE_INTERVAL_FRACTION: f32 = 0.25;

/// Units of a weather's transition byte per game hour of transition.
pub(super) const TRANSITION_BYTE_PER_HOUR: f32 = 16.0;
pub(super) const MIN_TRANSITION_HOURS: f32 = 0.25;
pub(super) const MAX_TRANSITION_HOURS: f32 = 3.0;

/// The frequency byte of the rarest lightning, and the real seconds between its strikes.
pub(super) const RAREST_STRIKE_BYTE: u8 = 255;
pub(super) const RAREST_STRIKE_INTERVAL_SECONDS: f32 = 60.0;
/// The frequency byte of the most frequent lightning, and the real seconds between its strikes.
pub(super) const MOST_FREQUENT_STRIKE_BYTE: u8 = 15;
pub(super) const MOST_FREQUENT_STRIKE_INTERVAL_SECONDS: f32 = 5.0;

/// How far from the player a strike lands.
pub(super) const NEAREST_STRIKE_DISTANCE_M: f32 = 300.0;
pub(super) const FARTHEST_STRIKE_DISTANCE_M: f32 = 3000.0;
pub(super) const SPEED_OF_SOUND_M_PER_S: f32 = 343.0;
/// The flash intensity of the farthest strike; the nearest flashes at 1.
pub(super) const FARTHEST_FLASH_INTENSITY: f32 = 0.35;
