use std::sync::Arc;

use glam::Mat4;

use super::rig::{CreatureRig, PackLoadError};
use crate::anim::{
    AnimPlayer, BindError, Clip, ClipBinding, ClipCursors, ClipError, Cycle, FiredMarker,
    PoseError, SampleError, Trs, local_to_model, sample,
};
use crate::rendering::frustum::Aabb;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharacterId(pub u32);

#[derive(Debug, thiserror::Error)]
pub enum PoseUpdateError {
    #[error(transparent)]
    Sample(#[from] SampleError),
    #[error(transparent)]
    Pose(#[from] PoseError),
}

#[derive(Debug, thiserror::Error)]
pub enum ClipSelectError {
    #[error(transparent)]
    Pack(#[from] PackLoadError),
    #[error("clip {clip} is out of range, pack '{pack}' lists {count} clips")]
    UnknownClip {
        pack: String,
        clip: usize,
        count: usize,
    },
    #[error("pack '{pack}' lists clip '{clip}' but does not contain it")]
    ClipMissingInPack { pack: String, clip: String },
    #[error(transparent)]
    Binding(#[from] BindError),
    #[error(transparent)]
    Looping(#[from] ClipError),
}

/// The clip being played. Debug playback always loops, whatever the clip's own
/// cycle says, so the converter's clamped clips are replayed seamlessly.
struct ActiveClip {
    pack: usize,
    name: String,
    clip: Clip,
    binding: ClipBinding,
    cursors: ClipCursors,
}

/// What drives a character's pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterKind {
    /// A creature playing clips.
    Creature,
    /// A static prop's own skin, posed from outside (ragdoll) rather than by clips.
    Prop,
}

pub struct CharacterInstance {
    pub id: CharacterId,
    rig: Arc<CreatureRig>,
    active: Option<ActiveClip>,
    player: AnimPlayer,
    local_pose: Vec<Trs>,
    model_pose: Vec<Mat4>,
    pose_bounds: Aabb,
    fired_markers: Vec<FiredMarker>,
    transform: Mat4,
    /// Set by a failed pose update, cleared by selecting a clip; a faulted character is not drawn.
    faulted: bool,
    pose_stale: bool,
    /// Pose or transform changed since the shadows last saw this character.
    shadow_dirty: bool,
    /// Pose or transform changed since the frame data (palette, instance) was last built.
    frame_dirty: bool,
    /// World bounds the shadows last saw, so a move also invalidates where it left.
    shadowed_bounds: Option<Aabb>,
    /// First matrix of this character in the frame's palette buffer.
    pub palette_offset: u32,
    kind: CharacterKind,
}

impl CharacterInstance {
    pub fn new(
        id: CharacterId,
        kind: CharacterKind,
        rig: Arc<CreatureRig>,
        transform: Mat4,
    ) -> Self {
        let model_pose = rig.rest_model_pose().to_vec();
        Self {
            id,
            active: None,
            player: AnimPlayer::new(0, 0.0, 1.0),
            local_pose: rig.skeleton.rest_pose().to_vec(),
            pose_bounds: rig.pose_bounds(&model_pose),
            model_pose,
            fired_markers: Vec::new(),
            rig,
            transform,
            faulted: false,
            pose_stale: false,
            shadow_dirty: true,
            frame_dirty: true,
            shadowed_bounds: None,
            palette_offset: 0,
            kind,
        }
    }

    pub fn kind(&self) -> CharacterKind {
        self.kind
    }

    /// Replaces the model-space pose; `pose` must hold one matrix per joint.
    pub fn set_model_pose(&mut self, pose: &[Mat4]) {
        if pose.len() != self.model_pose.len() {
            crate::utils::error_once(&format!("skin-pose-len:{}", self.rig.name), || {
                format!(
                    "skin: {} got a pose of {} joints, the rig has {}",
                    self.rig.name,
                    pose.len(),
                    self.model_pose.len()
                )
            });
            return;
        }
        self.model_pose.copy_from_slice(pose);
        self.pose_bounds = self.rig.pose_bounds(&self.model_pose);
        self.shadow_dirty = true;
        self.frame_dirty = true;
    }

    pub fn rig(&self) -> &CreatureRig {
        &self.rig
    }

    pub fn rig_handle(&self) -> &Arc<CreatureRig> {
        &self.rig
    }

    pub fn model_pose(&self) -> &[Mat4] {
        &self.model_pose
    }

    pub fn transform(&self) -> &Mat4 {
        &self.transform
    }

    pub fn set_transform(&mut self, transform: Mat4) {
        self.transform = transform;
        self.shadow_dirty = true;
        self.frame_dirty = true;
    }

    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    /// Pack index and clip name of the playing clip, `None` while the rest pose shows.
    pub fn active_clip(&self) -> Option<(usize, &str)> {
        self.active
            .as_ref()
            .map(|active| (active.pack, active.name.as_str()))
    }

    pub fn time(&self) -> f32 {
        self.player.time()
    }

    pub fn speed(&self) -> f32 {
        self.player.speed()
    }

    pub fn clip_duration(&self) -> Option<f32> {
        self.active.as_ref().map(|active| active.clip.duration())
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.player.set_speed(speed);
    }

    /// Plays clip `clip` of the creature's pack `pack`, as listed by the pack
    /// index, loading the pack file on first use. Nothing changes on failure.
    pub fn set_clip(&mut self, pack: usize, clip: usize) -> Result<(), ClipSelectError> {
        let packs = self.rig.packs();
        let listing = packs.get(pack).ok_or(PackLoadError::UnknownPack {
            index: pack,
            count: packs.len(),
        })?;
        let name = listing
            .clips
            .get(clip)
            .ok_or_else(|| ClipSelectError::UnknownClip {
                pack: listing.file.clone(),
                clip,
                count: listing.clips.len(),
            })?;
        let loaded = self.rig.load_pack(pack)?;
        let source = loaded
            .clip_named(name)
            .ok_or_else(|| ClipSelectError::ClipMissingInPack {
                pack: listing.file.clone(),
                clip: name.clone(),
            })?;
        let binding = ClipBinding::resolve(source, &self.rig.skeleton)?;
        binding.warn_unbound(&self.rig.name, name);
        let looped = Clip::new(
            source.name().to_owned(),
            source.duration(),
            Cycle::Loop,
            source.tracks().to_vec(),
            source.markers().to_vec(),
        )?;
        self.active = Some(ActiveClip {
            pack,
            name: name.clone(),
            cursors: ClipCursors::new(&looped),
            clip: looped,
            binding,
        });
        self.player = AnimPlayer::new(0, 0.0, self.player.speed());
        self.faulted = false;
        self.pose_stale = true;
        Ok(())
    }

    /// Advances the player and recomputes the model-space pose; a pose that
    /// did not move is left alone. A failure faults the character.
    pub fn update_pose(&mut self, dt: f32) -> Result<(), PoseUpdateError> {
        if self.faulted {
            return Ok(());
        }
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        let previous_time = self.player.time();
        self.fired_markers.clear();
        self.player
            .advance(dt, &active.clip, &mut self.fired_markers);
        #[allow(
            clippy::float_cmp,
            reason = "exact: an unchanged clock means the player did not advance"
        )]
        let time_unchanged = self.player.time() == previous_time;
        if time_unchanged && !self.pose_stale {
            return Ok(());
        }
        self.pose_stale = false;
        self.shadow_dirty = true;
        self.frame_dirty = true;

        self.local_pose
            .copy_from_slice(self.rig.skeleton.rest_pose());
        let sampled = sample(
            &active.clip,
            &active.binding,
            &mut active.cursors,
            self.player.time(),
            &mut self.local_pose,
        )
        .map_err(PoseUpdateError::from)
        .and_then(|()| {
            local_to_model(&self.rig.skeleton, &self.local_pose, &mut self.model_pose)
                .map_err(Into::into)
        });
        if let Err(error) = sampled {
            self.faulted = true;
            return Err(error);
        }
        self.pose_bounds = self.rig.pose_bounds(&self.model_pose);
        Ok(())
    }

    pub fn world_bounds(&self) -> Aabb {
        self.pose_bounds.transformed(&self.transform)
    }

    /// True once after the pose or transform changed: the frame data is stale.
    pub fn take_frame_dirty(&mut self) -> bool {
        std::mem::take(&mut self.frame_dirty)
    }

    /// Region cached shadows must redraw: where the character is now and where
    /// they last saw it. `None` while neither pose nor transform changed.
    pub fn take_shadow_region(&mut self) -> Option<Aabb> {
        if !self.shadow_dirty {
            return None;
        }
        self.shadow_dirty = false;
        let current = self.world_bounds();
        let previous = self.shadowed_bounds.replace(current);
        Some(previous.map_or(current, |previous| previous.union(&current)))
    }

    /// Everything this character may have left in cached shadows, for despawn.
    pub fn shadow_footprint(&self) -> Aabb {
        let current = self.world_bounds();
        self.shadowed_bounds
            .map_or(current, |previous| previous.union(&current))
    }

    pub fn write_palette(&self, out: &mut Vec<Mat4>) {
        self.rig.write_palette(&self.model_pose, out);
    }
}
