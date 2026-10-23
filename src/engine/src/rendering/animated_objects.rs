//! Runtime state of animated placed objects (doors, containers, activators,
//! furniture): which clip plays, the sampled pose, and the open/closed state.
//! Only objects with a running clip are ticked; a finished object keeps its
//! final pose without costing anything per frame.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use tracing::{debug, error, info};

use super::row_pose::{PoseSource, RowPose};
use super::skeleton_overlay::PosedSkeleton;
use crate::anim::{
    ActiveSet, AnimPlayer, AnimationRig, ClipCursors, FiredMarker, PoseError, SampleError, Trs,
    local_to_model, sample,
};
use crate::sim::{EntityId, KinematicPose, SimCommand};

/// Where in its loop a looping static starts, as a fraction of the clip: derived from the ref,
/// so neighbouring copies of one model do not run in lockstep and every load looks the same.
fn loop_phase(form_key: &str) -> f32 {
    let hash = xxhash_rust::xxh3::xxh3_64(form_key.as_bytes());
    (hash >> 40) as f32 / (1u64 << 24) as f32
}

/// Activators reuse the door states: Closed is off, Open is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorState {
    Closed,
    Opening,
    Open,
    Closing,
}

/// One animated REFR a freshly integrated cell hands to the runtime.
pub struct AnimatedRefr {
    pub entity: EntityId,
    pub model_path: String,
    pub form_key: String,
    pub world: Mat4,
    pub rig: Arc<AnimationRig>,
    /// Doors, containers, activators and furniture respond to activation;
    /// anything else only runs its looping sequence on its own.
    pub activatable: bool,
}

impl AnimatedRefr {
    /// Whether the runtime has anything to do with this object.
    pub fn is_runtime_object(&self) -> bool {
        self.activatable || self.rig.roles().auto_loop.is_some()
    }
}

pub enum AnimatedObjectEvent {
    Added(AnimatedRefr),
    Removed(EntityId),
    /// Every cell was torn down at once (worldspace or interior switch).
    Cleared,
}

#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("{0:?} is not a resident animated object")]
    NotResident(EntityId),
    #[error("{0}: the model has no clips to play")]
    NoClips(String),
    #[error("{0}: could not reset the pose before playing: {1}")]
    Pose(String, #[source] TickError),
}

#[derive(Debug, thiserror::Error)]
pub enum TickError {
    #[error(transparent)]
    Sample(#[from] SampleError),
    #[error(transparent)]
    Pose(#[from] PoseError),
}

pub struct AnimatedObjectState {
    refr: String,
    model: Arc<AnimationRig>,
    world: Mat4,
    /// Uniform REFR scale; kinematic bodies carry no scale.
    scale: f32,
    player: Option<AnimPlayer>,
    cursors: Option<ClipCursors>,
    local_pose: Vec<Trs>,
    model_pose: Vec<Mat4>,
    door_state: DoorState,
    /// What the playing clip ends in unless reversed again.
    target: DoorState,
    /// Responds to activation; false for statics that merely loop.
    activatable: bool,
}

impl AnimatedObjectState {
    fn new(refr: AnimatedRefr) -> Self {
        let (scale, _, _) = refr.world.to_scale_rotation_translation();
        let activatable = refr.activatable;
        Self {
            refr: refr.form_key,
            world: refr.world,
            scale: scale.x,
            local_pose: refr.rig.rest_local().to_vec(),
            model_pose: refr.rig.rest_model().to_vec(),
            model: refr.rig,
            player: None,
            cursors: None,
            door_state: DoorState::Closed,
            target: DoorState::Closed,
            activatable,
        }
    }

    /// Starts the looping sequence of a non-activatable object; false when it has none.
    fn start_auto_loop(&mut self) -> bool {
        let Some(clip) = self.model.roles().auto_loop else {
            return false;
        };
        let clip_data = &self.model.asset.clips[clip];
        let start = loop_phase(&self.refr) * clip_data.duration();
        self.player = Some(AnimPlayer::new(clip, start, 1.0));
        self.cursors = Some(ClipCursors::new(clip_data));
        true
    }

    /// Writes a pose clip's single state over the local pose and refreshes the model pose.
    fn apply_pose_clip(&mut self, clip: usize) -> Result<(), TickError> {
        let asset = &self.model.asset;
        let mut cursors = ClipCursors::new(&asset.clips[clip]);
        sample(
            &asset.clips[clip],
            self.model.binding(clip),
            &mut cursors,
            0.0,
            &mut self.local_pose,
        )?;
        local_to_model(&asset.skeleton, &self.local_pose, &mut self.model_pose)?;
        Ok(())
    }

    /// Resets the local pose to the closed state: the rest pose under the closed pose clip, if any.
    fn reset_to_closed(&mut self) -> Result<(), TickError> {
        self.local_pose.copy_from_slice(self.model.rest_local());
        match self.model.roles().closed_pose {
            Some(clip) => self.apply_pose_clip(clip),
            None => Ok(()),
        }
    }

    /// Puts a freshly loaded object into its closed pose; false when it has none to apply.
    fn settle_closed_pose(&mut self) -> bool {
        if self.model.roles().closed_pose.is_none() {
            return false;
        }
        match self.reset_to_closed() {
            Ok(()) => true,
            Err(error) => {
                error!(
                    "anim: {} keeps its rest pose, closed pose failed: {error}",
                    self.refr
                );
                self.local_pose.copy_from_slice(self.model.rest_local());
                self.model_pose.copy_from_slice(self.model.rest_model());
                false
            }
        }
    }

    fn start(&mut self, clip: usize, from_end: bool) -> Result<(), ActivationError> {
        let duration = self.model.asset.clips[clip].duration();
        let (time, speed) = if from_end {
            (duration, -1.0)
        } else {
            (0.0, 1.0)
        };
        info!(
            "anim: start {} {} speed={speed}",
            self.refr,
            self.model.asset.clips[clip].name()
        );
        self.reset_to_closed()
            .map_err(|error| ActivationError::Pose(self.refr.clone(), error))?;
        self.player = Some(AnimPlayer::new(clip, time, speed));
        self.cursors = Some(ClipCursors::new(&self.model.asset.clips[clip]));
        Ok(())
    }

    fn toggle(&mut self) -> Result<(), ActivationError> {
        let in_motion = self.is_in_motion();
        if let (Some(player), true) = (self.player.as_mut(), in_motion) {
            let reversed = -player.speed();
            player.set_speed(reversed);
            (self.door_state, self.target) = match self.door_state {
                DoorState::Opening => (DoorState::Closing, DoorState::Closed),
                _ => (DoorState::Opening, DoorState::Open),
            };
            info!("anim: reverse {} speed={reversed}", self.refr);
            return Ok(());
        }
        let roles = self.model.roles();
        let (open, close, opened_pose, closed_pose) = (
            roles.open,
            roles.close,
            roles.opened_pose,
            roles.closed_pose,
        );
        let no_clips = || ActivationError::NoClips(self.refr.clone());
        if self.door_state == DoorState::Closed {
            let clip = open.or(opened_pose).ok_or_else(no_clips)?;
            self.start(clip, false)?;
            (self.door_state, self.target) = (DoorState::Opening, DoorState::Open);
        } else {
            match (close, open, closed_pose) {
                (Some(close), _, _) => self.start(close, false)?,
                (None, Some(open), _) => self.start(open, true)?,
                (None, None, Some(closed)) => self.start(closed, false)?,
                (None, None, None) => return Err(no_clips()),
            }
            (self.door_state, self.target) = (DoorState::Closing, DoorState::Closed);
        }
        Ok(())
    }

    fn is_in_motion(&self) -> bool {
        matches!(self.door_state, DoorState::Opening | DoorState::Closing)
    }

    /// Advances the playing clip and refreshes the pose; true once it finished.
    fn advance(&mut self, dt: f32, fired: &mut Vec<FiredMarker>) -> Result<bool, TickError> {
        let (Some(player), Some(cursors)) = (&mut self.player, &mut self.cursors) else {
            return Ok(true);
        };
        let clip = &self.model.asset.clips[player.clip()];
        fired.clear();
        player.advance(dt, clip, fired);
        sample(
            clip,
            self.model.binding(player.clip()),
            cursors,
            player.time(),
            &mut self.local_pose,
        )?;
        local_to_model(
            &self.model.asset.skeleton,
            &self.local_pose,
            &mut self.model_pose,
        )?;

        for marker in fired
            .iter()
            .map(|fired| &clip.markers()[fired.marker_index])
        {
            debug!(
                "anim: marker {} {} {:?} at {:.3}",
                self.refr,
                clip.name(),
                marker.text,
                marker.time
            );
            let reached = match marker.text.trim().to_ascii_lowercase().as_str() {
                "opened" => Some(DoorState::Open),
                "closed" => Some(DoorState::Closed),
                _ => None,
            };
            if reached == Some(self.target) {
                self.door_state = self.target;
            }
        }
        let finished = player.is_finished();
        if finished {
            info!(
                "anim: done {} {} state={:?}",
                self.refr,
                clip.name(),
                self.target
            );
            self.door_state = self.target;
            self.player = None;
            self.cursors = None;
        }
        Ok(finished)
    }

    fn push_kinematic_poses(&self, entity: EntityId, out: &mut Vec<KinematicPose>) {
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return;
        }
        let unscale = Mat4::from_scale(Vec3::splat(1.0 / self.scale));
        let rest_inverse = self.model.rest_model_inverse();
        #[allow(
            clippy::needless_range_loop,
            reason = "joint indexes two parallel pose arrays and is also emitted as the joint id"
        )]
        for joint in 0..self.model_pose.len() {
            if !self.model.is_moving(joint as u16) {
                continue;
            }
            let delta = self.model_pose[joint] * rest_inverse[joint];
            let body = self.world * delta * unscale;
            let (_, rotation, translation) = body.to_scale_rotation_translation();
            out.push(KinematicPose {
                entity,
                joint: joint as u16,
                translation,
                rotation,
            });
        }
    }
}

#[derive(Default)]
pub struct AnimatedObjects {
    states: HashMap<EntityId, AnimatedObjectState>,
    active: ActiveSet<EntityId>,
    fired: Vec<FiredMarker>,
    /// Entities whose pose changed this frame, for the row writer.
    updated: Vec<EntityId>,
    kinematic: Vec<KinematicPose>,
    finished: Vec<EntityId>,
    /// Entities whose remembered kinematic poses the sim must forget.
    released: Vec<EntityId>,
    released_all: bool,
}

impl AnimatedObjects {
    pub fn apply_events(&mut self, events: Vec<AnimatedObjectEvent>) {
        let mut auto_loops = 0;
        for event in events {
            match event {
                AnimatedObjectEvent::Added(refr) => {
                    let entity = refr.entity;
                    self.active.remove(entity);
                    self.released.push(entity);
                    let mut state = AnimatedObjectState::new(refr);
                    let running = if state.activatable {
                        state.settle_closed_pose()
                    } else {
                        let started = state.start_auto_loop();
                        auto_loops += usize::from(started);
                        started
                    };
                    if running {
                        self.active.insert(entity);
                    }
                    self.states.insert(entity, state);
                }
                AnimatedObjectEvent::Removed(entity) => {
                    self.states.remove(&entity);
                    self.active.remove(entity);
                    self.released.push(entity);
                }
                AnimatedObjectEvent::Cleared => {
                    self.states.clear();
                    self.active = ActiveSet::new();
                    self.released.clear();
                    self.released_all = true;
                }
            }
        }
        if auto_loops > 0 {
            info!("auto-play loops: {auto_loops} refs");
        }
    }

    /// Whether the entity is an activatable animated object.
    pub fn contains(&self, entity: EntityId) -> bool {
        self.states
            .get(&entity)
            .is_some_and(|state| state.activatable)
    }

    /// Opens a closed object, closes an open one, reverses one mid-motion.
    pub fn toggle(&mut self, entity: EntityId) -> Result<(), ActivationError> {
        let state = self
            .states
            .get_mut(&entity)
            .filter(|state| state.activatable)
            .ok_or(ActivationError::NotResident(entity))?;
        state.toggle()?;
        self.active.insert(entity);
        Ok(())
    }

    /// Advances every playing clip by `dt`. Costs nothing when none plays.
    pub fn tick(&mut self, dt: f32) {
        self.updated.clear();
        self.kinematic.clear();
        if self.active.is_empty() {
            return;
        }
        self.finished.clear();
        for entity in self.active.iter() {
            let Some(state) = self.states.get_mut(&entity) else {
                self.finished.push(entity);
                continue;
            };
            match state.advance(dt, &mut self.fired) {
                Ok(finished) => {
                    self.updated.push(entity);
                    state.push_kinematic_poses(entity, &mut self.kinematic);
                    if finished {
                        self.finished.push(entity);
                    }
                }
                Err(error) => {
                    error!("anim: {} stopped: {error}", state.refr);
                    state.player = None;
                    state.cursors = None;
                    state.door_state = state.target;
                    self.finished.push(entity);
                }
            }
        }
        for entity in &self.finished {
            self.active.remove(*entity);
        }
    }

    /// This frame's world poses for the kinematic bodies; valid until the next tick.
    pub fn kinematic_poses(&self) -> &[KinematicPose] {
        &self.kinematic
    }

    /// Commands telling the sim to forget the poses of objects that were
    /// removed or replaced since the last call.
    pub fn take_kinematic_releases(&mut self) -> Vec<SimCommand> {
        let mut commands = Vec::new();
        if std::mem::take(&mut self.released_all) {
            commands.push(SimCommand::ReleaseAllKinematicPoses);
        }
        if !self.released.is_empty() {
            commands.push(SimCommand::ReleaseKinematicPoses(std::mem::take(
                &mut self.released,
            )));
        }
        commands
    }

    pub fn has_updates(&self) -> bool {
        !self.updated.is_empty()
    }

    /// Every resident animated object in its current pose, idle ones included.
    pub fn skeletons(&self) -> impl Iterator<Item = PosedSkeleton<'_>> {
        self.states.values().map(|state| PosedSkeleton {
            parents: state.model.asset.skeleton.parents(),
            model_pose: &state.model_pose,
            world: state.world,
            joint_reach: state.model.joint_reach(),
        })
    }

    pub fn poses(&self) -> AnimatedPoses<'_> {
        AnimatedPoses { objects: self }
    }
}

/// The poses of the objects that moved this frame.
pub struct AnimatedPoses<'a> {
    objects: &'a AnimatedObjects,
}

impl PoseSource for AnimatedPoses<'_> {
    fn len(&self) -> usize {
        self.objects.updated.len()
    }

    fn entity(&self, index: usize) -> EntityId {
        self.objects.updated[index]
    }

    fn pose(&self, index: usize) -> Option<RowPose<'_>> {
        let entity = *self.objects.updated.get(index)?;
        let Some(state) = self.objects.states.get(&entity) else {
            crate::utils::error_once(&format!("anim-pose-missing:{entity:?}"), || {
                format!(
                    "anim: {entity:?} was updated but has no state; its rows keep their last pose"
                )
            });
            return None;
        };
        Some(RowPose::Skeletal {
            refr_world: state.world,
            model_pose: &state.model_pose,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{AnimationAsset, Clip, Cycle, Interpolation, Keys, NodeTrack, Skeleton};
    use crate::sim::refr_entity_id;

    fn rig(clips: &[(&str, Cycle)]) -> Arc<AnimationRig> {
        let skeleton = Skeleton::new(
            vec!["node".to_owned()],
            vec![None],
            vec![Trs {
                translation: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::ONE,
            }],
        )
        .unwrap();
        let clips = clips
            .iter()
            .map(|(name, cycle)| {
                let track = NodeTrack {
                    node_name: "node".to_owned(),
                    translation: Some(
                        Keys::new(
                            vec![0.0, 1.0],
                            vec![Vec3::ZERO, Vec3::X],
                            Interpolation::Linear,
                        )
                        .unwrap(),
                    ),
                    rotation: None,
                    scale: None,
                };
                Clip::new((*name).to_owned(), 1.0, *cycle, vec![track], vec![]).unwrap()
            })
            .collect();
        let asset = AnimationAsset {
            skeleton,
            anim_joints: vec![],
            clips,
            skins: vec![],
            warnings: vec![],
        };
        Arc::new(AnimationRig::new(asset).unwrap())
    }

    fn placed(key: &str, rig: &Arc<AnimationRig>, activatable: bool) -> AnimatedObjectEvent {
        AnimatedObjectEvent::Added(AnimatedRefr {
            entity: refr_entity_id(key),
            model_path: "m.glb".to_owned(),
            form_key: key.to_owned(),
            world: Mat4::IDENTITY,
            rig: Arc::clone(rig),
            activatable,
        })
    }

    fn tick_for(objects: &mut AnimatedObjects, steps: u32) {
        for _ in 0..steps {
            objects.tick(0.1);
        }
    }

    fn x_of(objects: &AnimatedObjects, key: &str) -> f32 {
        objects.states[&refr_entity_id(key)].model_pose[0].w_axis.x
    }

    #[test]
    fn a_looping_static_advances_without_activation() {
        let rig = rig(&[("Spin", Cycle::Loop)]);
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("s", &rig, false)]);
        let phase = loop_phase("s");
        tick_for(&mut objects, 5);
        assert!((x_of(&objects, "s") - (phase + 0.5).fract()).abs() < 1e-3);
        tick_for(&mut objects, 7);
        assert!(
            (x_of(&objects, "s") - (phase + 1.2).fract()).abs() < 1e-3,
            "it wraps"
        );
        assert!(objects.has_updates());
    }

    #[test]
    fn looping_statics_start_at_a_phase_of_their_own_that_is_stable_per_ref() {
        let rig = rig(&[("Spin", Cycle::Loop)]);
        let start_of = |key: &str| {
            let mut objects = AnimatedObjects::default();
            objects.apply_events(vec![placed(key, &rig, false)]);
            objects.tick(0.0);
            x_of(&objects, key)
        };
        assert!((start_of("a") - start_of("a")).abs() < f32::EPSILON);
        assert!((start_of("a") - loop_phase("a")).abs() < 1e-3);
        assert!(
            (start_of("a") - start_of("b")).abs() > 1e-3,
            "two refs of one model are out of step"
        );
        assert!((0.0..1.0).contains(&loop_phase("anything")));
    }

    #[test]
    fn a_clamped_static_does_not_play() {
        let rig = rig(&[("Swing", Cycle::Clamp)]);
        let refr = AnimatedRefr {
            entity: refr_entity_id("s"),
            model_path: String::new(),
            form_key: "s".to_owned(),
            world: Mat4::IDENTITY,
            rig: Arc::clone(&rig),
            activatable: false,
        };
        assert!(!refr.is_runtime_object());
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("s", &rig, false)]);
        tick_for(&mut objects, 5);
        assert!(x_of(&objects, "s").abs() < 1e-6);
        assert!(!objects.has_updates());
    }

    #[test]
    fn loop_wins_over_clamp_when_both_exist() {
        let rig = rig(&[("Swing", Cycle::Clamp), ("Spin", Cycle::Loop)]);
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("s", &rig, false)]);
        tick_for(&mut objects, 5);
        assert!((x_of(&objects, "s") - (loop_phase("s") + 0.5).fract()).abs() < 1e-3);
    }

    #[test]
    fn a_looping_static_is_not_activatable() {
        let rig = rig(&[("Spin", Cycle::Loop)]);
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("s", &rig, false)]);
        assert!(!objects.contains(refr_entity_id("s")));
        assert!(matches!(
            objects.toggle(refr_entity_id("s")),
            Err(ActivationError::NotResident(_))
        ));
    }

    #[test]
    fn an_activatable_object_waits_for_activation() {
        let rig = rig(&[("Open", Cycle::Clamp), ("Spin", Cycle::Loop)]);
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("d", &rig, true)]);
        tick_for(&mut objects, 5);
        assert!(x_of(&objects, "d").abs() < 1e-6);
        assert!(objects.contains(refr_entity_id("d")));
        objects.toggle(refr_entity_id("d")).unwrap();
        tick_for(&mut objects, 5);
        assert!((x_of(&objects, "d") - 0.5).abs() < 1e-4);
    }

    #[test]
    fn removing_a_looping_static_stops_it() {
        let rig = rig(&[("Spin", Cycle::Loop)]);
        let mut objects = AnimatedObjects::default();
        objects.apply_events(vec![placed("s", &rig, false)]);
        objects.apply_events(vec![AnimatedObjectEvent::Removed(refr_entity_id("s"))]);
        tick_for(&mut objects, 5);
        assert!(!objects.has_updates());
    }
}
