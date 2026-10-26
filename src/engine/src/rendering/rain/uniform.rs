//! What the rain shader reads each frame: one uniform worked out from the weather's rain and
//! the camera, and the number of drops and splashes to draw. Everything is in engine units.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec2, Vec3};

use super::motion::{
    DropDrift, SplashPhase, StreakAxes, drop_count, fall_velocity, lattice_shift, splash_count,
};
use super::occlusion::{EXTENT, HEIGHT_RANGE};
use super::{RainInput, RainView};
use crate::bethesda_space::{ENGINE_UNITS_PER_METER, UNITS_TO_ENGINE};
use crate::utils::finite_or_zero;

/// The frames of the splash texture, side by side.
pub(super) const SPLASH_FRAMES: u32 = 16;
/// The edge of a splash's square.
const SPLASH_SIZE: f32 = 0.5 * ENGINE_UNITS_PER_METER;
/// How far from the camera splashes are placed.
const SPLASH_RADIUS: f32 = 25.0 * ENGINE_UNITS_PER_METER;
/// The rain never falls through a box smaller than this, whatever the weather asks for.
const MIN_BOX_EDGE: f32 = 4.0 * ENGINE_UNITS_PER_METER;
/// A drop is invisible at the first of these distances from the camera and fully visible at
/// the second.
const NEAR_FADE: [f32; 2] = [0.6 * ENGINE_UNITS_PER_METER, 3.0 * ENGINE_UNITS_PER_METER];
/// The share of half the box edge a drop is fully visible out to; beyond it the drop fades out
/// before it leaves the box on one side and enters it on the other.
const BOX_FADE_FROM: f32 = 0.6;
/// The same for a splash and the circle of [`SPLASH_RADIUS`].
const SPLASH_FADE_FROM: f32 = 0.7;

fn clean(vector: Vec3) -> Vec3 {
    Vec3::new(
        finite_or_zero(vector.x),
        finite_or_zero(vector.y),
        finite_or_zero(vector.z),
    )
}

/// How the drops of the weather fall, from the game units the weather names them in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Fall {
    /// Engine units per second.
    pub velocity: Vec3,
    /// The edge of the box the rain falls through, in engine units.
    pub edge: f32,
}

impl Fall {
    pub fn of(input: &RainInput) -> Self {
        let speed = finite_or_zero(input.fall_speed).max(0.0);
        let wind = Vec2::new(finite_or_zero(input.wind.x), finite_or_zero(input.wind.y));
        Self {
            velocity: fall_velocity(speed * UNITS_TO_ENGINE, wind * UNITS_TO_ENGINE),
            edge: (finite_or_zero(input.box_size) * UNITS_TO_ENGINE).max(MIN_BOX_EDGE),
        }
    }
}

/// Mirrors `RainUniform` in `rain.wgsl`, which says what each field holds.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct RainUniform {
    pub(super) along: [f32; 4],
    pub(super) across: [f32; 4],
    pub(super) lattice: [f32; 4],
    pub(super) tint: [f32; 4],
    pub(super) atlas: [f32; 4],
    pub(super) window: [f32; 4],
    pub(super) heights: [f32; 4],
    pub(super) fades: [f32; 4],
    pub(super) splash: [f32; 4],
    pub(super) splash_cycle: [u32; 4],
}

/// The uniform of a frame and how many drops and splashes it draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RainPlan {
    pub uniform: RainUniform,
    pub drops: u32,
    pub splashes: u32,
}

impl RainPlan {
    /// `drift` is where the drops have moved to by this frame.
    pub fn new(input: &RainInput, view: &RainView, fall: &Fall, drift: &DropDrift) -> Self {
        let intensity = finite_or_zero(input.intensity).clamp(0.0, 1.0);
        let width = finite_or_zero(input.size.x).max(0.0) * ENGINE_UNITS_PER_METER;
        let length = finite_or_zero(input.size.y).max(0.0) * ENGINE_UNITS_PER_METER;
        let has_streaks = width > 0.0 && length > 0.0;
        let axes = StreakAxes::new(fall.velocity, clean(view.forward));
        let eye = clean(view.eye);
        let shift = lattice_shift(drift.offset(), eye, fall.edge);

        let splash_edge = 2.0 * SPLASH_RADIUS;
        let splash_shift = lattice_shift(DVec3::ZERO, eye, splash_edge);
        let splash_phase = SplashPhase::at(if view.time.is_finite() {
            view.time.max(0.0)
        } else {
            0.0
        });
        let tint = clean(input.tint).max(Vec3::ZERO);

        let (window, heights) = match view.window {
            Some(window) => (
                [window.origin().x, window.origin().y, EXTENT, 1.0],
                [
                    window.bottom(),
                    HEIGHT_RANGE,
                    f32::from(u8::from(input.occludable)),
                    0.0,
                ],
            ),
            None => ([0.0, 0.0, EXTENT, 0.0], [0.0, HEIGHT_RANGE, 0.0, 0.0]),
        };
        let with = |vector: Vec3, last: f32| [vector.x, vector.y, vector.z, last];
        Self {
            uniform: RainUniform {
                along: with(axes.along, length),
                across: with(axes.across, width),
                lattice: with(shift, fall.edge),
                tint: with(tint, 0.0),
                atlas: [
                    input.subtextures.x.max(1) as f32,
                    input.subtextures.y.max(1) as f32,
                    SPLASH_FRAMES as f32,
                    SPLASH_SIZE,
                ],
                window,
                heights,
                fades: [NEAR_FADE[0], NEAR_FADE[1], BOX_FADE_FROM, SPLASH_FADE_FROM],
                splash: [
                    splash_phase.fraction,
                    splash_edge,
                    splash_shift.x,
                    splash_shift.z,
                ],
                splash_cycle: [splash_phase.cycle, 0, 0, 0],
            },
            drops: if has_streaks {
                drop_count(intensity, input.density)
            } else {
                0
            },
            splashes: if view.window.is_some() {
                splash_count(intensity)
            } else {
                0
            },
        }
    }
}
