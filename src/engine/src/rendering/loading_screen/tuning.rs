//! Every tunable of the viewer's control of a loading screen's model, in one place. None of
//! them is documented by the game: they are inferred and meant to be tuned by playing.

/// Degrees the model turns per pixel of horizontal drag: a drag across a 1000 px wide window
/// turns it by 315 degrees.
pub const DEGREES_PER_PIXEL: f32 = 0.315;

/// Zoom gained per pixel of upward drag; 200 px of drag span a zoom range of 1.
pub const ZOOM_PER_PIXEL: f32 = 0.005;

/// Degrees per second the model turns by itself, except while dragged and for the resume delay
/// after.
pub const SPIN_DEGREES_PER_SECOND: f32 = 10.0;

/// How far, in half windows per second, a held pan key moves the model: 1 crosses from the
/// middle to the edge of the pan window in a second.
pub const PAN_HALF_WINDOWS_PER_SECOND: f32 = 1.0;

/// The longest time one update may advance the model by, so a stalled frame does not make it jump.
pub const MAX_STEP_SECONDS: f32 = 0.25;
