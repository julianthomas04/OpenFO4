//! Footstep timings read from the markers of the converted first-person clips.
//!
//! The player has no animation of its own, so the step clock replays the foot marks of the clips
//! the vanilla player would play: the run, sprint and sneak cycles and the jump take-off and
//! landing clips. Clip identifiers below are asset names, not game values.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

use super::clip::Clip;
use super::gltf_import::{AnimationAsset, ImportError};
use crate::footsteps::{FootTag, GaitCycle, GaitTimings, GaitTimingsError};

/// Folder of the first-person clip packs below the asset root.
pub const FIRST_PERSON_ASSETS_DIR: &str = "meshes/Actors/Character/_1stPerson/CharacterAssets";
const ANIM_INDEX_FILE: &str = "skeleton.anims.json";

const RUN_CLIP: &str = "Common/RunForwardReady";
const SPRINT_CLIP: &str = "H2H/SprintForward";
const JUMP_UP_CLIP: &str = "H2H/JumpStart";
const JUMP_DOWN_CLIP: &str = "H2H/JumpLand";
/// The sneak cycle is the same in every weapon pack; the first pack alphabetically stands in.
const SNEAK_CLIP: &str = "44Pistol/WPNWalkForwardReady";

const RUN_MARKS: [FootTag; 2] = [FootTag::FootRight, FootTag::FootLeft];
const SPRINT_MARKS: [FootTag; 2] = [FootTag::FootSprintRight, FootTag::FootSprintLeft];

#[derive(Debug, Error)]
pub enum MarkerTimelineError {
    #[error("animation index '{path}' does not exist; run the installer's animation conversion")]
    IndexMissing { path: String },
    #[error("animation index '{path}' cannot be read: {source}")]
    IndexUnreadable {
        path: String,
        source: std::io::Error,
    },
    #[error("animation index '{path}' is malformed: {source}")]
    IndexMalformed {
        path: String,
        source: serde_json::Error,
    },
    #[error("clip '{clip}' is not listed in the animation index")]
    ClipNotIndexed { clip: String },
    #[error("clip pack '{path}' does not exist")]
    PackMissing { path: String },
    #[error("clip pack '{path}': {source}")]
    Import { path: String, source: ImportError },
    #[error("clip pack '{path}' does not contain clip '{clip}'")]
    ClipNotInPack { path: String, clip: String },
    #[error("clip '{clip}' has no {tag:?} marker")]
    MarkMissing { clip: String, tag: FootTag },
    #[error("clip '{clip}' has more than one {tag:?} marker")]
    MarkDuplicated { clip: String, tag: FootTag },
    #[error(transparent)]
    Timings(#[from] GaitTimingsError),
}

#[derive(Debug, Deserialize)]
struct IndexedPack {
    file: String,
    clips: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AnimIndex {
    packs: Vec<IndexedPack>,
}

/// Reads clips by name through the converter's index, importing each pack once.
struct ClipLibrary {
    dir: String,
    packs: Vec<IndexedPack>,
    loaded: HashMap<String, Vec<Clip>>,
}

impl ClipLibrary {
    fn open(dir: &str) -> Result<Self, MarkerTimelineError> {
        let path = format!("{dir}/{ANIM_INDEX_FILE}");
        let text = std::fs::read_to_string(&path).map_err(|source| match source.kind() {
            ErrorKind::NotFound => MarkerTimelineError::IndexMissing { path: path.clone() },
            _ => MarkerTimelineError::IndexUnreadable {
                path: path.clone(),
                source,
            },
        })?;
        let index: AnimIndex = serde_json::from_str(&text)
            .map_err(|source| MarkerTimelineError::IndexMalformed { path, source })?;
        Ok(Self {
            dir: dir.to_owned(),
            packs: index.packs,
            loaded: HashMap::new(),
        })
    }

    fn clip(&mut self, name: &str) -> Result<&Clip, MarkerTimelineError> {
        let file = self
            .packs
            .iter()
            .find(|pack| pack.clips.iter().any(|clip| clip == name))
            .map(|pack| pack.file.clone())
            .ok_or_else(|| MarkerTimelineError::ClipNotIndexed {
                clip: name.to_owned(),
            })?;
        let path = format!("{}/{file}", self.dir);
        if !self.loaded.contains_key(&file) {
            if !Path::new(&path).is_file() {
                return Err(MarkerTimelineError::PackMissing { path });
            }
            let clips = AnimationAsset::load(&path)
                .map_err(|source| MarkerTimelineError::Import {
                    path: path.clone(),
                    source,
                })?
                .clips;
            self.loaded.insert(file.clone(), clips);
        }
        self.loaded[&file]
            .iter()
            .find(|clip| clip.name() == name)
            .ok_or(MarkerTimelineError::ClipNotInPack {
                path,
                clip: name.to_owned(),
            })
    }
}

/// The time of the one marker of `tag` in the clip.
fn mark_time(clip: &Clip, tag: FootTag) -> Result<f32, MarkerTimelineError> {
    let mut times = clip
        .markers()
        .iter()
        .filter(|marker| FootTag::parse(&marker.text) == Some(tag))
        .map(|marker| marker.time);
    let time = times
        .next()
        .ok_or_else(|| MarkerTimelineError::MarkMissing {
            clip: clip.name().to_owned(),
            tag,
        })?;
    if times.next().is_some() {
        return Err(MarkerTimelineError::MarkDuplicated {
            clip: clip.name().to_owned(),
            tag,
        });
    }
    Ok(time)
}

fn cycle_of(
    library: &mut ClipLibrary,
    what: &'static str,
    clip_name: &str,
    tags: [FootTag; 2],
) -> Result<GaitCycle, MarkerTimelineError> {
    let clip = library.clip(clip_name)?;
    let marks = tags
        .into_iter()
        .map(|tag| Ok((mark_time(clip, tag)?, tag)))
        .collect::<Result<Vec<_>, MarkerTimelineError>>()?;
    Ok(GaitCycle::new(what, clip.duration(), marks)?)
}

/// Builds the footstep timings from the clips in `dir` (the first-person `CharacterAssets`
/// folder). A missing clip pack, clip or marker is an error.
pub fn load_gait_timings(dir: &str) -> Result<GaitTimings, MarkerTimelineError> {
    let mut library = ClipLibrary::open(dir)?;
    let run = cycle_of(&mut library, "run", RUN_CLIP, RUN_MARKS)?;
    let sprint = cycle_of(&mut library, "sprint", SPRINT_CLIP, SPRINT_MARKS)?;
    let sneak = cycle_of(&mut library, "sneak", SNEAK_CLIP, RUN_MARKS)?;
    let jump_up = mark_time(library.clip(JUMP_UP_CLIP)?, FootTag::JumpUp)?;
    let jump_down = mark_time(library.clip(JUMP_DOWN_CLIP)?, FootTag::JumpDown)?;
    Ok(GaitTimings::new(run, sprint, sneak, jump_up, jump_down)?)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;
    use crate::footsteps::Gait;

    /// A one-node glTF whose one clip has the given extras.
    fn pack_glb(clips: &[(&str, &str)]) -> Vec<u8> {
        let mut bin = Vec::new();
        for value in [0.0_f32, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0] {
            bin.extend(value.to_le_bytes());
        }
        let animations: Vec<String> = clips
            .iter()
            .map(|(name, extras)| {
                format!(
                    r#"{{"name":"{name}","extras":{extras},"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}],"samplers":[{{"input":0,"output":1,"interpolation":"LINEAR"}}]}}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":{}}}],"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":8}},{{"buffer":0,"byteOffset":8,"byteLength":24}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":2,"type":"SCALAR"}},{{"bufferView":1,"componentType":5126,"count":2,"type":"VEC3"}}],"nodes":[{{"name":"root"}}],"animations":[{}]}}"#,
            bin.len(),
            animations.join(",")
        );
        let mut json = json.into_bytes();
        json.resize(json.len().next_multiple_of(4), b' ');
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend(b"glTF");
        out.extend(2_u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        out
    }

    fn extras(stop: f32, annotations: &[(f32, &str)]) -> String {
        let list: Vec<String> = annotations
            .iter()
            .map(|(t, text)| format!(r#"{{"t":{t},"text":"{text}"}}"#))
            .collect();
        format!(r#"{{"stop":{stop},"annotations":[{}]}}"#, list.join(","))
    }

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        /// Every clip the loader needs, with the real marks of the vanilla clips; `edit` may
        /// change one clip's extras by name.
        fn new(edit: impl Fn(&str, String) -> String) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let packs: [(&str, &[(&str, String)]); 3] = [
                (
                    "Common.anims.glb",
                    &[(
                        RUN_CLIP,
                        extras(0.667, &[(0.3, "FootRight"), (0.633, "FootLeft")]),
                    )],
                ),
                (
                    "H2H.anims.glb",
                    &[
                        (
                            SPRINT_CLIP,
                            extras(
                                0.6,
                                &[(0.133, "FootSprintRight"), (0.467, "FootSprintLeft")],
                            ),
                        ),
                        (JUMP_UP_CLIP, extras(0.5, &[(0.033, "JumpUp")])),
                        (JUMP_DOWN_CLIP, extras(0.5, &[(0.067, "JumpDown")])),
                    ],
                ),
                (
                    "44Pistol.anims.glb",
                    &[(
                        SNEAK_CLIP,
                        extras(1.0, &[(0.367, "FootRight"), (0.867, "FootLeft")]),
                    )],
                ),
            ];
            let mut index = Vec::new();
            for (file, clips) in packs {
                let edited: Vec<(&str, String)> = clips
                    .iter()
                    .map(|(name, extras)| (*name, edit(name, extras.clone())))
                    .collect();
                let refs: Vec<(&str, &str)> =
                    edited.iter().map(|(n, e)| (*n, e.as_str())).collect();
                std::fs::write(dir.path().join(file), pack_glb(&refs)).unwrap();
                let names: Vec<String> = edited.iter().map(|(n, _)| format!("\"{n}\"")).collect();
                index.push(format!(
                    r#"{{"file":"{file}","clips":[{}]}}"#,
                    names.join(",")
                ));
            }
            std::fs::write(
                dir.path().join(ANIM_INDEX_FILE),
                format!(
                    r#"{{"skeleton":"skeleton.glb","packs":[{}]}}"#,
                    index.join(",")
                ),
            )
            .unwrap();
            Self { dir }
        }

        fn path(&self) -> &str {
            self.dir.path().to_str().unwrap()
        }
    }

    #[test]
    fn timings_follow_the_clip_markers_and_durations() {
        let fixture = Fixture::new(|_, extras| extras);
        let timings = load_gait_timings(fixture.path()).unwrap();
        let run = timings.cycle(Gait::Run).unwrap();
        assert_eq!(run.period(), 0.667);
        assert_eq!(
            run.marks(),
            &[(0.3, FootTag::FootRight), (0.633, FootTag::FootLeft)]
        );
        let sprint = timings.cycle(Gait::Sprint).unwrap();
        assert_eq!(sprint.period(), 0.6);
        assert_eq!(
            sprint.marks(),
            &[
                (0.133, FootTag::FootSprintRight),
                (0.467, FootTag::FootSprintLeft)
            ]
        );
        let sneak = timings.cycle(Gait::Sneak).unwrap();
        assert_eq!(sneak.period(), 1.0);
        assert_eq!(
            sneak.marks(),
            &[(0.367, FootTag::FootRight), (0.867, FootTag::FootLeft)]
        );
        assert_eq!(timings.jump_up_offset(), 0.033);
        assert_eq!(timings.jump_down_offset(), 0.067);
    }

    #[test]
    fn unrelated_annotations_are_ignored() {
        let fixture = Fixture::new(|name, extras| {
            if name == RUN_CLIP {
                self::extras(
                    0.667,
                    &[
                        (0.1, "weaponSwing"),
                        (0.3, "FootRight"),
                        (0.633, "FootLeft"),
                    ],
                )
            } else {
                extras
            }
        });
        assert!(load_gait_timings(fixture.path()).is_ok());
    }

    #[test]
    fn a_missing_mark_names_the_clip_and_tag() {
        let fixture = Fixture::new(|name, extras| {
            if name == RUN_CLIP {
                self::extras(0.667, &[(0.3, "FootRight")])
            } else {
                extras
            }
        });
        let err = load_gait_timings(fixture.path()).unwrap_err();
        assert!(
            matches!(&err, MarkerTimelineError::MarkMissing { clip, tag }
                if clip == RUN_CLIP && *tag == FootTag::FootLeft),
            "{err}"
        );
    }

    #[test]
    fn a_duplicated_mark_is_rejected() {
        let fixture = Fixture::new(|name, extras| {
            if name == JUMP_UP_CLIP {
                self::extras(0.5, &[(0.033, "JumpUp"), (0.2, "JumpUp")])
            } else {
                extras
            }
        });
        let err = load_gait_timings(fixture.path()).unwrap_err();
        assert!(matches!(
            err,
            MarkerTimelineError::MarkDuplicated {
                tag: FootTag::JumpUp,
                ..
            }
        ));
    }

    #[test]
    fn a_mark_outside_the_cycle_is_a_timings_error() {
        let fixture = Fixture::new(|name, extras| {
            if name == SPRINT_CLIP {
                self::extras(
                    0.3,
                    &[(0.133, "FootSprintRight"), (0.467, "FootSprintLeft")],
                )
            } else {
                extras
            }
        });
        assert!(matches!(
            load_gait_timings(fixture.path()),
            Err(MarkerTimelineError::Timings(
                GaitTimingsError::MarkOutOfCycle { .. }
            ))
        ));
    }

    #[test]
    fn a_missing_index_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_gait_timings(dir.path().to_str().unwrap()),
            Err(MarkerTimelineError::IndexMissing { .. })
        ));
    }

    #[test]
    fn a_clip_absent_from_the_index_is_reported() {
        let fixture = Fixture::new(|_, extras| extras);
        std::fs::write(
            fixture.dir.path().join(ANIM_INDEX_FILE),
            r#"{"skeleton":"skeleton.glb","packs":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            load_gait_timings(fixture.path()),
            Err(MarkerTimelineError::ClipNotIndexed { clip }) if clip == RUN_CLIP
        ));
    }

    #[test]
    fn a_missing_pack_file_is_reported() {
        let fixture = Fixture::new(|_, extras| extras);
        std::fs::remove_file(fixture.dir.path().join("H2H.anims.glb")).unwrap();
        assert!(matches!(
            load_gait_timings(fixture.path()),
            Err(MarkerTimelineError::PackMissing { .. })
        ));
    }

    /// Reads the converted clips of an installed game; the expected values are the vanilla marks.
    #[test]
    #[ignore = "needs the converted first-person animations in the install"]
    fn the_real_installation_yields_the_vanilla_timings() {
        let assets = crate::assets::resolve_asset_root(
            &crate::assets::AssetRootEnv::from_process(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        )
        .expect("set OPENFO4_INSTALL_DIR to a converted installation");
        let dir = assets.join(FIRST_PERSON_ASSETS_DIR);
        let timings = load_gait_timings(dir.to_str().unwrap()).unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 0.0015;
        let run = timings.cycle(Gait::Run).unwrap();
        assert!(close(run.period(), 0.667), "{run:?}");
        assert!(close(run.marks()[0].0, 0.300) && run.marks()[0].1 == FootTag::FootRight);
        assert!(close(run.marks()[1].0, 0.633) && run.marks()[1].1 == FootTag::FootLeft);
        let sprint = timings.cycle(Gait::Sprint).unwrap();
        assert!(close(sprint.marks()[0].0, 0.133), "{sprint:?}");
        assert!(close(sprint.marks()[1].0, 0.467), "{sprint:?}");
        let sneak = timings.cycle(Gait::Sneak).unwrap();
        assert!(close(sneak.period(), 1.0), "{sneak:?}");
        assert!(close(sneak.marks()[0].0, 0.367), "{sneak:?}");
        assert!(close(sneak.marks()[1].0, 0.867), "{sneak:?}");
        assert!(close(timings.jump_up_offset(), 0.033));
        assert!(close(timings.jump_down_offset(), 0.067));
    }
}
