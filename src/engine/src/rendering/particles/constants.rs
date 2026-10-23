//! Tunable values for semantics the particle data does not pin down. Each is
//! provisional: tune by in-game comparison.

/// Longest simulation step; longer frame times are clamped so a hitch does not
/// make particles teleport or spawn in one burst.
pub const MAX_STEP_SECONDS: f32 = 1.0 / 15.0;

/// Systems simulated at once in a world; further spawns are refused.
pub const MAX_ACTIVE_SYSTEMS: usize = 256;

/// Drag `percentage` is treated as a continuous decay rate per second:
/// velocity along the axis is multiplied by `exp(-percentage * scale * dt)`.
pub const DRAG_RATE_PER_SECOND: f32 = 1.0;

/// Wind has no direction in the data; each particle is pushed along its own
/// fixed horizontal direction with `strength * scale` eu/s^2.
pub const WIND_ACCELERATION_PER_STRENGTH: f32 = 1.0;

/// The LOD modifier's begin/end are fractions of this camera distance (eu).
pub const LOD_REFERENCE_DISTANCE: f32 = 1000.0;

/// Sub-texture `frame_count` is playback frames per second, times this factor.
pub const SUBTEXTURE_FRAMES_PER_COUNT_SECOND: f32 = 1.0;

/// Emitter and particle `radius` is the quad half-extent, times this factor.
pub const RADIUS_TO_HALF_EXTENT: f32 = 1.0;

/// Spherical gravity pulls towards the gravity object; `decay` shrinks the
/// pull as `exp(-decay * distance)`.
pub const SPHERICAL_GRAVITY_ATTRACTS: bool = true;

/// Attachments farther than this from the camera (eu) do not spawn their particle systems.
/// No fire or smoke system fades out farther than this, so beyond it nothing would be seen.
pub const SPAWN_RADIUS_EU: f32 = 4096.0;

/// A live attachment keeps running until it is this much (eu) beyond [`SPAWN_RADIUS_EU`], so
/// one standing at the edge does not respawn every time the camera shakes.
pub const DESPAWN_HYSTERESIS_EU: f32 = 512.0;

/// A cap-refused attachment only evicts a live one that is at least this much (eu) farther
/// from the camera, so two at similar distances do not trade places every frame.
pub const EVICTION_MARGIN_EU: f32 = 256.0;
