//! Footstep timings read from the markers of the converted first-person clips.
//!
//! The player has no animation of its own, so the step clock replays the foot marks of the clips
//! the vanilla player would play: the run, sprint and sneak cycles and the jump take-off and
//! landing clips. Clip identifiers below are asset names, not game values.

use thiserror::Error;

use super::clip::Clip;
use super::clip_library::{ClipLibrary, ClipLibraryError};
use crate::footsteps::{FootTag, GaitCycle, GaitTimings, GaitTimingsError};

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
    #[error(transparent)]
    Clips(#[from] ClipLibraryError),
    #[error("clip '{clip}' has no {tag:?} marker")]
    MarkMissing { clip: String, tag: FootTag },
    #[error("clip '{clip}' has more than one {tag:?} marker")]
    MarkDuplicated { clip: String, tag: FootTag },
    #[error(transparent)]
    Timings(#[from] GaitTimingsError),
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

/// Builds the footstep timings from the clips of the first-person character folder. A missing
/// clip pack, clip or marker is an error.
pub fn load_gait_timings(library: &mut ClipLibrary) -> Result<GaitTimings, MarkerTimelineError> {
    let run = cycle_of(library, "run", RUN_CLIP, RUN_MARKS)?;
    let sprint = cycle_of(library, "sprint", SPRINT_CLIP, SPRINT_MARKS)?;
    let sneak = cycle_of(library, "sneak", SNEAK_CLIP, RUN_MARKS)?;
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
    use crate::anim::character_assets::{
        ANIM_INDEX_FILE, AnimationFileError, FIRST_PERSON_ASSETS_DIR,
    };
    use crate::anim::test_glb::{TranslationClip, translation_pack_glb};
    use crate::footsteps::Gait;

    /// A one-node glTF whose one clip has the given extras.
    fn pack_glb(clips: &[(&str, &str)]) -> Vec<u8> {
        let clips: Vec<TranslationClip<'_>> = clips
            .iter()
            .map(|&(name, extras)| TranslationClip {
                name,
                extras,
                node: 0,
                times: [0.0, 1.0],
                keys: [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
            })
            .collect();
        translation_pack_glb(r#"{"name":"root"}"#, &clips)
    }

    fn extras(stop: f32, annotations: &[(f32, &str)]) -> String {
        let list: Vec<String> = annotations
            .iter()
            .map(|(t, text)| format!(r#"{{"t":{t},"text":"{text}"}}"#))
            .collect();
        format!(r#"{{"stop":{stop},"annotations":[{}]}}"#, list.join(","))
    }

    const UNIT_DIR: &str = "meshes/Unit";

    struct Fixture {
        root: tempfile::TempDir,
    }

    impl Fixture {
        /// Every clip the loader needs, with the real marks of the vanilla clips; `edit` may
        /// change one clip's extras by name.
        fn new(edit: impl Fn(&str, String) -> String) -> Self {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join(UNIT_DIR);
            std::fs::create_dir_all(&dir).unwrap();
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
                std::fs::write(dir.join(file), pack_glb(&refs)).unwrap();
                let names: Vec<String> = edited.iter().map(|(n, _)| format!("\"{n}\"")).collect();
                index.push(format!(
                    r#"{{"file":"{file}","clips":[{}]}}"#,
                    names.join(",")
                ));
            }
            std::fs::write(
                dir.join(ANIM_INDEX_FILE),
                format!(
                    r#"{{"skeleton":"skeleton.glb","packs":[{}]}}"#,
                    index.join(",")
                ),
            )
            .unwrap();
            Self { root }
        }

        fn unit(&self) -> std::path::PathBuf {
            self.root.path().join(UNIT_DIR)
        }

        fn library(&self) -> ClipLibrary {
            ClipLibrary::open(self.root.path().to_str().unwrap(), UNIT_DIR).unwrap()
        }
    }

    #[test]
    fn timings_follow_the_clip_markers_and_durations() {
        let fixture = Fixture::new(|_, extras| extras);
        let timings = load_gait_timings(&mut fixture.library()).unwrap();
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
        assert!(load_gait_timings(&mut fixture.library()).is_ok());
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
        let err = load_gait_timings(&mut fixture.library()).unwrap_err();
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
        let err = load_gait_timings(&mut fixture.library()).unwrap_err();
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
            load_gait_timings(&mut fixture.library()),
            Err(MarkerTimelineError::Timings(
                GaitTimingsError::MarkOutOfCycle { .. }
            ))
        ));
    }

    #[test]
    fn a_clip_absent_from_the_index_is_reported() {
        let fixture = Fixture::new(|_, extras| extras);
        std::fs::write(
            fixture.unit().join(ANIM_INDEX_FILE),
            r#"{"skeleton":"skeleton.glb","packs":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            load_gait_timings(&mut fixture.library()),
            Err(MarkerTimelineError::Clips(ClipLibraryError::ClipNotIndexed { clip }))
                if clip == RUN_CLIP
        ));
    }

    #[test]
    fn a_missing_pack_file_is_reported() {
        let fixture = Fixture::new(|_, extras| extras);
        std::fs::remove_file(fixture.unit().join("H2H.anims.glb")).unwrap();
        assert!(matches!(
            load_gait_timings(&mut fixture.library()),
            Err(MarkerTimelineError::Clips(ClipLibraryError::Pack(
                AnimationFileError::Missing { .. }
            )))
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
        let mut library =
            ClipLibrary::open(assets.to_str().unwrap(), FIRST_PERSON_ASSETS_DIR).unwrap();
        let timings = load_gait_timings(&mut library).unwrap();
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
