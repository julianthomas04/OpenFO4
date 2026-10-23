//! Fallout 4's wading water displacement ("ripples"): a 512x512 height field
//! around the player, stirred by actors wading and by things hitting the
//! water, that the water shader reads as `t2`; see `docs/engine/rendering/surfaces-and-effects.md`,
//! "Displacement".
//!
//! [`state`] is the CPU driver as a pure state machine, [`gpu`] runs its plan,
//! [`wading`] decides which actors splat. [`WaterDisplacement`] ties them to
//! the engine: it turns the player, the water world and the physics water
//! entries into a frame of splats.

mod gpu;
pub mod state;
pub mod wading;

use std::time::{Duration, Instant};

use glam::{Vec2, Vec3, Vec4};

pub use gpu::{DisplacementGpu, REQUIRED_FORMAT_FEATURES, TEXTURE_RESOLUTION};
use state::{DisplacementDriver, FrameInput, QUAD_SIZE, impact_in_range};

use crate::bethesda_space::{UNITS_FROM_ENGINE, position_from_engine};
use crate::rendering::gpu_profiler::GpuProfiler;
use crate::sim::PlayerBody;
use crate::utils::env_flag;

/// The water frame constants register the quad centre and size travel in: the
/// first `cb12` slot the shader's registers leave unused (`water_wrapper.wgsl` reads it).
pub const QUAD_REGISTER: usize = 36;
/// The `ps_cb2` row of a displacement quad draw's per-draw constants that
/// [`WaterDisplacement::quad_draw_register`] fills.
pub const DISPLACEMENT_QUAD_DRAW_REGISTER: usize = 0;
const TOGGLE_ENV: &str = "OPENFO4_WATER_DISPLACEMENT";
const TRACE_INTERVAL: Duration = Duration::from_secs(1);
/// The handle the player wades under in the actor map.
const PLAYER_HANDLE: u64 = crate::sim::PLAYER_ENTITY.0;

/// What the engine knows this frame, in engine space.
pub struct EngineFrame<'a> {
    pub dt: f32,
    /// The camera's position: the origin of the field only while no player
    /// body is given (free flight).
    pub reference: Vec3,
    /// The spawned player's body; it wades when its feet are in the water.
    pub player: Option<PlayerBody>,
    /// Where bodies entered the water since the last frame.
    pub water_entries: &'a [Vec3],
}

impl EngineFrame<'_> {
    /// The quad's reference point: the player actor's own xy (not the camera), which
    /// re-snaps the quad and origins every splat. The camera stands in while no player
    /// body is spawned.
    fn origin(&self) -> Vec3 {
        self.player.map_or(self.reference, |body| body.feet)
    }
}

fn game_xy(engine: Vec3) -> Vec2 {
    position_from_engine(engine).truncate()
}

/// What `OPENFO4_WATER_TRACE` reports about the displacement.
#[derive(Default)]
struct TraceTally {
    frames: u32,
    splats: usize,
    steps: u32,
    clears: u32,
    was_active: bool,
    impacts_received: usize,
    impacts_in_range: usize,
    /// Distance (game units) of the nearest impact received, in the interval.
    nearest_impact: Option<f32>,
    /// The last wading verdict of the spawned player: feet, surface and top
    /// heights in game units, `None` while no player body is spawned.
    player: Option<PlayerWading>,
}

#[derive(Copy, Clone)]
struct PlayerWading {
    feet: f32,
    top: f32,
    surface: Option<f32>,
    wading: bool,
}

impl std::fmt::Display for PlayerWading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "feet {:.0} top {:.0} water {:?} wading {}",
            self.feet, self.top, self.surface, self.wading
        )
    }
}

pub struct WaterDisplacement {
    enabled: bool,
    trace: bool,
    driver: DisplacementDriver,
    gpu: DisplacementGpu,
    actors: Vec<(u64, Option<Vec2>)>,
    impacts: Vec<Vec2>,
    tally: TraceTally,
    last_trace: Instant,
}

impl WaterDisplacement {
    pub fn new(device: &wgpu::Device, trace: bool) -> Self {
        Self {
            enabled: env_flag(TOGGLE_ENV, true),
            trace,
            driver: DisplacementDriver::new(),
            gpu: DisplacementGpu::new(device),
            actors: Vec::new(),
            impacts: Vec::new(),
            tally: TraceTally::default(),
            last_trace: Instant::now(),
        }
    }

    pub fn targets(&self) -> &[wgpu::TextureView; 2] {
        self.gpu.views()
    }

    /// The target the water shader reads this frame, or `None` while the field
    /// is idle or the feature is off: the surfaces then see the neutral field,
    /// which is what the cleared field would show.
    pub fn visible_target(&self) -> Option<usize> {
        let state = self.driver.state();
        (self.enabled && self.driver.plan().active && state.is_live())
            .then(|| state.current_target())
    }

    /// Whether the feature runs; when off nothing consumes the water entries.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The water vertex shader's `cb12[QUAD_REGISTER]`: the quad centre (game xy), the
    /// reciprocal quad size.
    pub fn quad_constants(&self) -> Vec4 {
        let centre = self.driver.state().centre();
        Vec4::new(centre.x, centre.y, QUAD_SIZE.recip(), 0.0)
    }

    /// `ps_cb2[0]` of a displacement quad draw (`water_wrapper.wgsl`): the marker, the
    /// quad centre relative to the camera (game xy) and the reciprocal size.
    pub fn quad_draw_register(&self, camera_game_xy: Vec2) -> Vec4 {
        let relative = self.driver.state().centre() - camera_game_xy;
        Vec4::new(1.0, relative.x, relative.y, QUAD_SIZE.recip())
    }

    /// Runs the driver for this frame. `water_height` answers the water
    /// surface height (engine units) at an engine-space point.
    pub fn update(&mut self, frame: &EngineFrame<'_>, water_height: impl Fn(Vec3) -> Option<f32>) {
        if !self.enabled {
            return;
        }
        let origin = frame.origin();
        self.actors.clear();
        self.actors.push((
            PLAYER_HANDLE,
            frame
                .player
                .filter(|body| player_wades(body, &water_height))
                .map(|_| game_xy(origin)),
        ));
        if self.trace {
            self.record_inputs(frame, &water_height);
        }
        self.impacts.clear();
        self.impacts
            .extend(frame.water_entries.iter().copied().map(game_xy));
        self.driver.plan_frame(&FrameInput {
            dt: frame.dt,
            player: game_xy(origin),
            actors: &self.actors,
            impacts: &self.impacts,
        });
        self.record_trace();
    }

    pub fn upload(&self, queue: &wgpu::Queue) {
        self.gpu.upload(queue, self.driver.plan());
    }

    /// Encodes the frame's passes and only then lets the state advance, so a
    /// frame whose passes are never encoded cannot desynchronise it from the
    /// GPU's field.
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, profiler: &GpuProfiler) {
        self.gpu.encode(encoder, profiler, self.driver.plan());
        self.driver.commit();
    }

    fn record_inputs(
        &mut self,
        frame: &EngineFrame<'_>,
        water_height: &impl Fn(Vec3) -> Option<f32>,
    ) {
        let player_xy = game_xy(frame.origin());
        for &entry in frame.water_entries {
            let distance = (game_xy(entry) - player_xy).length();
            self.tally.impacts_received += 1;
            self.tally.impacts_in_range += usize::from(impact_in_range(game_xy(entry), player_xy));
            self.tally.nearest_impact = Some(
                self.tally
                    .nearest_impact
                    .map_or(distance, |nearest| nearest.min(distance)),
            );
        }
        self.tally.player = frame.player.map(|body| {
            let surface = water_height(body.feet).map(|height| height * UNITS_FROM_ENGINE);
            PlayerWading {
                feet: body.feet.y * UNITS_FROM_ENGINE,
                top: (body.feet.y + body.height) * UNITS_FROM_ENGINE,
                surface,
                wading: player_wades(&body, water_height),
            }
        });
    }

    fn record_trace(&mut self) {
        if !self.trace {
            return;
        }
        let plan = self.driver.plan();
        let tally = &mut self.tally;
        tally.frames += 1;
        tally.splats += plan.splats.len();
        tally.steps += plan.steps;
        tally.clears += u32::from(plan.cleared);
        if plan.cleared {
            tracing::info!("water-displacement: field cleared to neutral");
        }
        if tally.was_active != plan.active {
            tracing::info!(
                "water-displacement: {}",
                if plan.active {
                    "woke (a splat arrived)"
                } else {
                    "idle for 10 s, stopped"
                }
            );
            tally.was_active = plan.active;
        }
        if self.last_trace.elapsed() < TRACE_INTERVAL {
            return;
        }
        self.last_trace = Instant::now();
        let centre = self.driver.state().centre();
        let was_active = self.tally.was_active;
        let tally = std::mem::replace(
            &mut self.tally,
            TraceTally {
                was_active,
                ..TraceTally::default()
            },
        );
        tracing::info!(
            "water-displacement: frames={} splats={} steps={} clears={} impacts received={} in range={} nearest={:?} player=[{}] active={} idle={:.1}s centre=({:.0},{:.0}) visible_target={:?}",
            tally.frames,
            tally.splats,
            tally.steps,
            tally.clears,
            tally.impacts_received,
            tally.impacts_in_range,
            tally.nearest_impact,
            tally
                .player
                .map_or_else(|| "none spawned".to_owned(), |player| player.to_string()),
            self.driver.plan().active,
            self.driver.state().idle_seconds(),
            centre.x,
            centre.y,
            self.visible_target(),
        );
    }
}

/// Whether the player's feet are in the water and its top above it.
fn player_wades(body: &PlayerBody, water_height: &impl Fn(Vec3) -> Option<f32>) -> bool {
    water_height(body.feet).is_some_and(|surface| {
        wading::is_wading(
            surface * UNITS_FROM_ENGINE,
            body.feet.y * UNITS_FROM_ENGINE,
            body.height * UNITS_FROM_ENGINE,
        )
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exactly representable results"
)]
mod tests {
    use super::*;

    fn body(feet_y: f32) -> PlayerBody {
        PlayerBody {
            feet: Vec3::new(0.0, feet_y, 0.0),
            height: 12.6,
        }
    }

    #[test]
    fn the_player_wades_when_the_surface_is_between_the_feet_and_the_head() {
        let surface = |_| Some(1.0);
        assert!(player_wades(&body(0.0), &surface));
        assert!(!player_wades(&body(2.0), &surface));
        assert!(!player_wades(&body(-20.0), &surface));
    }

    #[test]
    fn no_water_under_the_player_is_not_wading() {
        assert!(!player_wades(&body(0.0), &|_| None));
    }

    #[test]
    fn the_field_is_centred_on_the_player_body_not_the_camera() {
        let camera = Vec3::new(5.0, 1.7, 5.0);
        let mut frame = EngineFrame {
            dt: 0.016,
            reference: camera,
            player: Some(body(0.0)),
            water_entries: &[],
        };
        assert_eq!(frame.origin(), Vec3::ZERO);
        frame.player = None;
        assert_eq!(frame.origin(), camera);
    }

    #[test]
    fn engine_positions_convert_to_game_xy() {
        // Engine (x, y, z) is game (x, -z, y), ten game units per engine unit.
        assert_eq!(game_xy(Vec3::new(1.0, 5.0, -2.0)), Vec2::new(10.0, 20.0));
    }
}
