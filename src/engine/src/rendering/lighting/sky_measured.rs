//! How vanilla Fallout 4 shows its sky colours, as the probe measured it (no fog, image space
//! fixed at exposure 1.5 and sky scale 1).

/// The sky's light for a colour, relative to the fully fogged fog colour of the same bytes by
/// day: a grey sky ladder (bytes 48 to 255) fits `0.489 x (byte / 255)^2.2` within 3 % per level,
/// and the sky keeps it at every hour from 00:00 to 20:30, while the fog's fill drops to 0.48 at
/// night.
pub const SKY_BRIGHTNESS: f32 = 0.489;

/// The light at the centre of the sun disc for its weather Sun colour, on the same scale: the
/// brightest disc pixel adds `0.49 x (byte / 255)^2.2` at Sun bytes 16 and 48 alike, unclipped.
pub const SUN_DISC_BRIGHTNESS: f32 = 0.49;

/// The light of a fully opaque star texel for the weather Stars colour, on the same scale: the
/// probe's stars at Stars 255 add a mean of 0.558 times the star texture's mean decoded texel
/// over the sky they cover (0.39 to 0.71 by which part of the texture shows). That spans the
/// sky's own factor, which the clouds (0.473 against 0.470) and the sun disc (0.49) take too.
pub const STAR_BRIGHTNESS: f32 = SKY_BRIGHTNESS;

/// How many `fSunBaseSize` units make one degree of the edge of the sun disc sprite. The
/// vanilla disc (425 units) shows its half-peak light over a disc of radius 1.92 and 2.18 degrees
/// in two poses, and the vanilla `Sun_d` texture's colour times alpha is above half its peak over
/// 9.3 % of the edge across: an edge of 41 to 47 degrees, 9.1 to 10.3 units per degree.
pub const SUN_DISC_SIZE_UNITS_PER_DEGREE: f32 = 9.6;

/// The edge in degrees of the disc sprite of a sun base size `size`.
pub fn sun_disc_edge_deg(size: f32) -> f32 {
    size / SUN_DISC_SIZE_UNITS_PER_DEGREE
}
