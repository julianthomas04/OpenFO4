//! What the rain shader reads each frame: one uniform worked out from the weather's rain, the
//! camera and the window the occlusion map shows, and the number of drops and splashes to draw.
//! Everything is in engine units.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec2, Vec3};

use super::assets::SPLASH_FRAMES;
use super::motion::{
    DropDrift, SplashPhase, StreakAxes, drop_count, fall_velocity, lattice_shift, splash_count,
};
use super::occlusion::{EXTENT, HEIGHT_RANGE, OcclusionWindow};
use super::tuning::{
    BOX_FADE_FROM, LEAST_VARIATION, MIN_BOX_EDGE, NEAR_FADE, SPLASH_FADE_FROM, SPLASH_OPACITY,
    SPLASH_RADIUS, SPLASH_SIZE, STREAK_LENGTH_SCALE, STREAK_OPACITY, STREAK_WIDTH_SCALE,
};
use super::{RainInput, RainView};
use crate::bethesda_space::{ENGINE_UNITS_PER_METER, UNITS_TO_ENGINE};
use crate::utils::finite_or_zero;

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

/// The size of a streak in engine units: x its length along the fall and y its width across it.
/// The particle size of the weather names them the same way round, in metres before the scales
/// of the tuning; a size that is not a number or not positive gives zero.
fn streak_size(input: &RainInput) -> Vec2 {
    let size = Vec2::new(
        finite_or_zero(input.size.x).max(0.0) * STREAK_LENGTH_SCALE,
        finite_or_zero(input.size.y).max(0.0) * STREAK_WIDTH_SCALE,
    );
    size * ENGINE_UNITS_PER_METER
}

/// How many drops and splashes a frame draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Draws {
    pub drops: u32,
    pub splashes: u32,
}

impl Draws {
    pub const NONE: Self = Self {
        drops: 0,
        splashes: 0,
    };

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// What the weather and the camera ask for: nothing while it does not rain, the camera is
    /// under water or is no point in space; no drops for a weather that gives them no size.
    pub fn wanted(input: &RainInput, view: &RainView) -> Self {
        if view.underwater || !view.eye.is_finite() {
            return Self::NONE;
        }
        let intensity = finite_or_zero(input.intensity).clamp(0.0, 1.0);
        let size = streak_size(input);
        let has_streaks = size.x > 0.0 && size.y > 0.0;
        Self {
            drops: if has_streaks {
                drop_count(intensity, input.density)
            } else {
                0
            },
            splashes: splash_count(intensity),
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
    pub(super) streak_tint: [f32; 4],
    pub(super) atlas: [f32; 4],
    pub(super) window: [f32; 4],
    pub(super) heights: [f32; 4],
    pub(super) fades: [f32; 4],
    pub(super) looks: [f32; 4],
    pub(super) splash: [f32; 4],
    pub(super) splash_cycle: [u32; 4],
}

impl RainUniform {
    /// `window` is what the occlusion map shows and `drift` where the drops have moved to by
    /// this frame.
    pub fn new(
        input: &RainInput,
        view: &RainView,
        window: OcclusionWindow,
        fall: &Fall,
        drift: &DropDrift,
    ) -> Self {
        let size = streak_size(input);
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

        let with = |vector: Vec3, last: f32| [vector.x, vector.y, vector.z, last];
        Self {
            along: with(axes.along, size.x),
            across: with(axes.across, size.y),
            lattice: with(shift, fall.edge),
            tint: with(tint, 0.0),
            streak_tint: with(tint * input.streak_material.base_color, 0.0),
            atlas: [
                input.subtextures.x.max(1) as f32,
                input.subtextures.y.max(1) as f32,
                SPLASH_FRAMES as f32,
                SPLASH_SIZE,
            ],
            window: [
                window.origin().x,
                window.origin().y,
                EXTENT,
                window.bottom(),
            ],
            heights: [
                HEIGHT_RANGE,
                f32::from(u8::from(input.occludable)),
                0.0,
                0.0,
            ],
            fades: [NEAR_FADE[0], NEAR_FADE[1], BOX_FADE_FROM, SPLASH_FADE_FROM],
            looks: [STREAK_OPACITY, SPLASH_OPACITY, LEAST_VARIATION, 0.0],
            splash: [
                splash_phase.fraction,
                splash_edge,
                splash_shift.x,
                splash_shift.z,
            ],
            splash_cycle: [splash_phase.cycle, 0, 0, 0],
        }
    }
}
