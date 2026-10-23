//! Dev hook (`OPENFO4_AUTO_CHARACTER="<creature>:<pack>/<clip>[;...]"`): once the
//! player has spawned, spawn each listed skinned character in front of the
//! camera, side by side, looping the given clip. Exercises GPU skinning where
//! OS input cannot be injected into the window.

use std::collections::VecDeque;

use glam::Mat4;
use tracing::{debug, error, info};

use super::anim_debug_panel::{SPAWN_DISTANCE, ground_position};
use super::camera::Camera;
use super::skinning::character::CharacterId;
use super::skinning::rig::ClipPackEntry;
use super::skinning::{CharacterSystem, SpawnRequest, rig::CreatureRig};
use crate::sim::SimWorld;

const ENV_VAR: &str = "OPENFO4_AUTO_CHARACTER";
/// Gap between neighbouring characters, engine units (about 1.4 m).
const GAP: f32 = 10.0;
/// Share of the view width the line of characters may fill before it is moved back.
const VIEW_FILL: f32 = 0.8;

struct CharacterSpec {
    creature: String,
    pack: String,
    clip: String,
}

#[derive(Debug, thiserror::Error)]
enum SpecError {
    #[error("entry '{0}' is not '<creature>:<pack>/<clip>'")]
    Malformed(String),
    #[error("unknown creature '{0}'")]
    UnknownCreature(String),
    #[error("creature '{creature}' has no clip pack '{pack}'")]
    UnknownPack { creature: String, pack: String },
    #[error("clip pack '{pack}' of '{creature}' has no clip '{clip}'")]
    UnknownClip {
        creature: String,
        pack: String,
        clip: String,
    },
}

fn parse_spec(entry: &str) -> Result<CharacterSpec, SpecError> {
    let malformed = || SpecError::Malformed(entry.to_owned());
    let (creature, source) = entry.split_once(':').ok_or_else(malformed)?;
    let (pack, clip) = source.split_once('/').ok_or_else(malformed)?;
    let [creature, pack, clip] = [creature, pack, clip].map(str::trim);
    if creature.is_empty() || pack.is_empty() || clip.is_empty() {
        return Err(malformed());
    }
    Ok(CharacterSpec {
        creature: creature.to_owned(),
        pack: pack.to_owned(),
        clip: clip.to_owned(),
    })
}

/// A pack matches by its file name or by the part of it before the first dot.
/// A clip matches as given relative to the pack or as the full stored name
/// (index clip names are paths below Animations/, e.g. `H2H/Idle`).
fn resolve_clip(
    packs: &[ClipPackEntry],
    spec: &CharacterSpec,
) -> Result<(usize, usize), SpecError> {
    let pack = packs
        .iter()
        .position(|entry| {
            entry.file == spec.pack || entry.file.split('.').next() == Some(spec.pack.as_str())
        })
        .ok_or_else(|| SpecError::UnknownPack {
            creature: spec.creature.clone(),
            pack: spec.pack.clone(),
        })?;
    let prefixed = format!("{}/{}", spec.pack, spec.clip);
    let clip = packs[pack]
        .clips
        .iter()
        .position(|name| *name == spec.clip || *name == prefixed)
        .ok_or_else(|| SpecError::UnknownClip {
            creature: spec.creature.clone(),
            pack: spec.pack.clone(),
            clip: spec.clip.clone(),
        })?;
    Ok((pack, clip))
}

pub(super) struct AutoCharacters {
    queue: VecDeque<CharacterSpec>,
    /// Spawn queued and awaiting `CharacterSystem::update`: the spec and the
    /// id reserved for the character.
    in_flight: Option<(CharacterSpec, CharacterId)>,
    /// Spawned characters left to right with their footprint, engine units.
    placed: Vec<(CharacterId, f32)>,
}

impl AutoCharacters {
    pub fn from_env() -> Self {
        let raw = crate::utils::optional_env_var(ENV_VAR).unwrap_or_default();
        let mut queue = VecDeque::new();
        for entry in raw
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            match parse_spec(entry) {
                Ok(spec) => queue.push_back(spec),
                Err(error) => error!("{ENV_VAR}: {error}"),
            }
        }
        Self {
            queue,
            in_flight: None,
            placed: Vec::new(),
        }
    }

    /// Spawns one character per call and sets its clip on the call after, once
    /// the character system has applied the spawn.
    pub fn tick(
        &mut self,
        ready: impl FnOnce() -> bool,
        characters: &mut CharacterSystem,
        sim: &SimWorld,
        camera: &Camera,
    ) {
        if let Some((spec, id)) = self.in_flight.take() {
            self.finish(&spec, id, characters, sim, camera);
            return;
        }
        if self.queue.is_empty() || !ready() {
            return;
        }
        let Some(spec) = self.queue.pop_front() else {
            return;
        };
        let Some(creature) = characters
            .creatures()
            .iter()
            .position(|def| def.name == spec.creature)
        else {
            error!("{ENV_VAR}: {}", SpecError::UnknownCreature(spec.creature));
            return;
        };
        let position = ground_position(sim, camera, SPAWN_DISTANCE, 0.0);
        let id = characters.queue_spawn(SpawnRequest {
            creature,
            transform: Mat4::from_translation(position),
        });
        self.in_flight = Some((spec, id));
    }

    fn finish(
        &mut self,
        spec: &CharacterSpec,
        id: CharacterId,
        characters: &mut CharacterSystem,
        sim: &SimWorld,
        camera: &Camera,
    ) {
        let Some(character) = characters.character_mut(id) else {
            let cause = characters.spawn_error().unwrap_or("spawn was not applied");
            error!("{ENV_VAR}: could not spawn '{}': {cause}", spec.creature);
            return;
        };
        self.placed.push((id, footprint(character.rig())));
        let outcome = resolve_clip(character.rig().packs(), spec)
            .map(|(pack, clip)| character.set_clip(pack, clip));
        self.line_up(characters, sim, camera);
        match outcome {
            Ok(Ok(())) => info!(
                "skin: auto-spawned {} clip={}/{}",
                spec.creature, spec.pack, spec.clip
            ),
            Ok(Err(error)) => error!(
                "{ENV_VAR}: clip {}/{} of '{}' failed: {error}",
                spec.pack, spec.clip, spec.creature
            ),
            Err(error) => error!("{ENV_VAR}: {error}"),
        }
    }
}

impl AutoCharacters {
    /// Centers the spawned characters left to right in front of the camera, far
    /// enough back that the line fits the view, and logs where each one stands.
    fn line_up(&mut self, characters: &mut CharacterSystem, sim: &SimWorld, camera: &Camera) {
        self.placed
            .retain(|&(id, _)| characters.character(id).is_some());
        if self.placed.is_empty() {
            return;
        }
        let total = self.placed.iter().map(|(_, width)| width).sum::<f32>()
            + GAP * (self.placed.len() - 1) as f32;
        let half_view_tan = (camera.fovy.to_radians() * 0.5).tan() * camera.aspect;
        let distance = SPAWN_DISTANCE.max(total * 0.5 / (half_view_tan * VIEW_FILL));
        debug!(
            "skin: auto-character layout eye={:?} forward={:?} distance={distance:.1} total_width={total:.1}",
            camera.eye,
            camera.forward()
        );
        let mut left_edge = -total * 0.5;
        for &(id, width) in &self.placed {
            let position = ground_position(sim, camera, distance, left_edge + width * 0.5);
            left_edge += width + GAP;
            let Some(character) = characters.character_mut(id) else {
                continue;
            };
            character.set_transform(Mat4::from_translation(position));
            info!(
                "skin: auto-character {} #{} at {position:?} ({:+.1} above eye)",
                character.rig().name,
                id.0,
                position.y - camera.eye.y
            );
        }
    }
}

/// The rest pose's larger horizontal extent, engine units: wide enough to
/// keep the next character clear of this one whichever way it faces.
fn footprint(rig: &CreatureRig) -> f32 {
    let bounds = rig.pose_bounds(rig.rest_model_pose());
    let size = bounds.max - bounds.min;
    size.x.max(size.z)
}
