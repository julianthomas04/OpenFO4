//! Command and input types: pure data (glam only — no rapier, no winit),
//! with one deliberate exception: `AddCellColliders` carries an opaque
//! sim-owned shape bake (`sim::bake::BakedCellColliders`) next to its
//! pure-data batch. The batch stays the source of truth — collider
//! registration is a deterministic derivation from the content DB, not a
//! gameplay mutation, so the bake is just an accelerator.
//! Everything that mutates the sim flows through [`SimCommand`].

use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};

use super::bake::BakedCellColliders;
use super::collision::CellColliderBatch;
use super::pipboy::PipBoyAction;
use super::ragdoll_tuning::RagdollSettings;
use super::{CellGrid, EntityId};
use crate::weather::WeatherId;

pub enum SimCommand {
    /// Register a streamed cell's static colliders (terrain + objects). The
    /// batch is built on the streaming worker thread; the sim turns it into
    /// rapier colliders on the main thread, preferring the worker's
    /// pre-baked shapes (`baked`) and only building on a cache+bake miss.
    /// While the sim holds its fold (`SimWorld::set_fold_hold`) the add is
    /// queued and inserted by `SimWorld::drain_pending_colliders`.
    AddCellColliders {
        grid: CellGrid,
        batch: Arc<CellColliderBatch>,
        baked: Option<BakedCellColliders>,
    },
    /// Drop every collider a cell registered (unload or LOD demotion). Also
    /// despawns any dynamic rigid bodies the cell owned.
    RemoveCellColliders { grid: CellGrid },
    /// Move the dynamic-physics simulation center/radius (the streamer pushes
    /// this every frame from the player's cell + debug-menu radius). Bodies
    /// within `radius` (Chebyshev cells) simulate; the rest freeze.
    SetPhysicsCenter { grid: CellGrid, radius: i32 },
    /// Spawn the player into freshly-streamed content once its collision is
    /// resident. In a worldspace the sim snaps onto the first surface below
    /// `pos`; in an interior it places the player at `pos` and lets gravity
    /// settle the capsule onto the floor (no snap).
    SpawnPlayer { pos: Vec3 },
    /// Reposition the player (freecam fly->walk handoff): snap straight down
    /// onto the first collider of any group below `pos`.
    TeleportPlayer { pos: Vec3 },
    /// Latest input snapshot, consumed by each fixed step until replaced.
    SetPlayerInput(PlayerInput),
    /// Enter/leave interior mode. Interiors have no LAND terrain — only object-
    /// mesh floors — so while enabled the spawn readiness gate is "the
    /// interior's collision is installed" instead of the per-grid ground-ready
    /// gate (the whole interior loads as one resident cell), and entry places
    /// the player without a ground-snap (gravity settles them onto the floor).
    /// Set by the debug interior view around its teleport; cleared on any
    /// worldspace switch.
    SetInteriorMode(bool),
    /// The renderer dropped these animated objects: forget their remembered poses.
    ReleaseKinematicPoses(Vec<EntityId>),
    /// The renderer dropped every animated object.
    ReleaseAllKinematicPoses,
    /// New ragdoll sleep tuning, applied to live and future ragdolls.
    SetRagdollSettings(RagdollSettings),
    /// An action from the Pip-Boy screen; a rejection is recorded on the model, never dropped.
    PipBoy(PipBoyAction),
    /// A change to the world clock; the sim rejects an invalid value with a warning and leaves
    /// the clock as it was.
    SetWorldClock(ClockEdit),
    /// A change to the weather cycle; the sim rejects an invalid one with a warning and leaves
    /// the cycle as it was.
    SetWeather(WeatherEdit),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClockEdit {
    /// The hour of the day, `0..=24`; the date and the days passed stay.
    SetHour(f32),
    SetPaused(bool),
    /// Game minutes per real minute.
    SetTimeScale(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WeatherEdit {
    /// Switches to the weather at once, ending any transition.
    Force(WeatherId),
    /// Starts the transition to the weather, as a roll would.
    Transition(WeatherId),
    /// How far the running transition has come, `0..=1`.
    SetProgress(f32),
    /// A strike of the current weather on the next frame, whether or not it has thunder.
    TriggerLightning,
}

/// World pose for the kinematic body of one animated joint of a placed
/// reference. Poses carry no scale (shapes are baked at the REFR's scale).
#[derive(Clone, Copy, Debug)]
pub struct KinematicPose {
    pub entity: EntityId,
    pub joint: u16,
    pub translation: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PlayerInput {
    /// x = strafe right, y = forward; unit length or zero, player-local.
    pub move_dir: Vec2,
    /// Camera yaw in radians, `CameraController` convention: yaw of -PI/2
    /// faces -Z, forward on the XZ plane = (cos yaw, sin yaw).
    pub yaw: f32,
    /// Held (Godot semantics: held while grounded triggers the jump).
    pub jump: bool,
    /// Desired states, NOT raw key holds: the input layer resolves the FO4
    /// toggle semantics (Shift arms sprint only while W alone is down and
    /// drops it on Shift/W-release/strafe/sneak; Ctrl toggles crouch, cleared
    /// by sprint start, untouched by jumping). The sim treats these as
    /// intent — `crouch` waits on the stand-up headroom probe, `sprint` only
    /// applies once not crouching.
    pub sprint: bool,
    pub crouch: bool,
}

impl PlayerInput {
    /// The same camera heading and stance, with every movement intent released.
    #[must_use]
    pub fn without_movement(self) -> Self {
        Self {
            move_dir: Vec2::ZERO,
            jump: false,
            sprint: false,
            ..self
        }
    }
}
