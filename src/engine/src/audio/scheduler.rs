//! Decides what a music type plays next. Pure: randomness, conditions and missing files are inputs.
//!
//! A type lists tracks (duplicates kept, so a repeated entry is proportionally more likely). The
//! tracks whose conditions hold and whose file is present are the eligible entries; one is picked
//! uniformly. A palette is a set of playlists that run at once: each layer plays its eligible
//! sub-tracks one after another in record order, a silent sub-track being a timed gap. An
//! ineligible sub-track is skipped, and a layer without any sound is dropped. A palette left with
//! no sound at all becomes a silent plan of its duration.

use std::collections::{HashMap, HashSet};

use super::conditions::{ConditionContext, ConditionError, evaluate};
use crate::assets::db::music::{MusicTrack, MusicTrackKind, MusicType};
use crate::random::RandomSource;

pub const FLAG_ABRUPT_TRANSITION: u32 = 0x02;
pub const FLAG_CYCLE_TRACKS: u32 = 0x04;

/// The tracing target of everything the world music decides and plays.
pub const LOG_TARGET: &str = "openfo4::audio::world";

#[derive(Debug, Clone, PartialEq)]
pub enum PlayPlan {
    Single {
        file_key: String,
    },
    Silent {
        seconds: f32,
    },
    Palette {
        layers: Vec<Vec<LayerStep>>,
        duration: f32,
        fade_out: Option<f32>,
    },
}

/// One entry of a palette layer's playlist.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerStep {
    Sound { file_key: String },
    Gap { seconds: f32 },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SchedulerError {
    #[error("music track {0} is listed but has no music track record")]
    UnknownTrack(String),
    #[error("palette sub-track {0} is itself a palette")]
    NestedPalette(String),
    #[error(transparent)]
    Condition(#[from] ConditionError),
}

#[derive(Debug, Clone, Default)]
pub struct TrackScheduler {
    missing_files: HashSet<String>,
}

impl TrackScheduler {
    /// `missing_files` are file keys known to be absent; tracks that need them never play.
    pub fn new(missing_files: HashSet<String>) -> Self {
        Self { missing_files }
    }

    /// The file keys known to be absent.
    pub fn missing_files(&self) -> &HashSet<String> {
        &self.missing_files
    }

    /// An abrupt type starts at once when it becomes the playing type; any other waits for the
    /// music that plays to end.
    pub fn is_abrupt(&self, music_type: &MusicType) -> bool {
        music_type.flags & FLAG_ABRUPT_TRANSITION != 0
    }

    /// A cycling type asks for a new plan whenever the previous one ends.
    pub fn cycles(&self, music_type: &MusicType) -> bool {
        music_type.flags & FLAG_CYCLE_TRACKS != 0
    }

    /// `None` means nothing is eligible: silence.
    pub fn next_plan(
        &self,
        music_type: &MusicType,
        tracks: &HashMap<String, MusicTrack>,
        ctx: &dyn ConditionContext,
        rng: &mut dyn RandomSource,
    ) -> Result<Option<PlayPlan>, SchedulerError> {
        let eligible = self.eligible(&music_type.tracks, tracks, ctx)?;
        if eligible.is_empty() {
            tracing::debug!(
                target: LOG_TARGET,
                music_type = music_type.editor_id.as_deref().unwrap_or(&music_type.form_key),
                listed = music_type.tracks.len(),
                "no eligible track"
            );
            return Ok(None);
        }
        let (_, track) = eligible[rng.below(eligible.len())];
        let plan = self.plan(track, tracks, ctx)?;
        tracing::debug!(
            target: LOG_TARGET,
            music_type = music_type.editor_id.as_deref().unwrap_or(&music_type.form_key),
            track = track.editor_id.as_deref().unwrap_or(&track.form_key),
            eligible = eligible.len(),
            plan = ?plan,
            "track picked"
        );
        Ok(Some(plan))
    }

    fn eligible<'a>(
        &self,
        keys: &'a [String],
        tracks: &'a HashMap<String, MusicTrack>,
        ctx: &dyn ConditionContext,
    ) -> Result<Vec<(&'a str, &'a MusicTrack)>, SchedulerError> {
        let mut eligible = Vec::new();
        for key in keys {
            let track = tracks
                .get(key)
                .ok_or_else(|| SchedulerError::UnknownTrack(key.clone()))?;
            if self.file_present(track) && evaluate(&track.conditions, ctx)? {
                eligible.push((key.as_str(), track));
            }
        }
        Ok(eligible)
    }

    fn file_present(&self, track: &MusicTrack) -> bool {
        match &track.kind {
            MusicTrackKind::Single { file_key, .. } => !self.missing_files.contains(file_key),
            MusicTrackKind::Silent { .. } | MusicTrackKind::Palette { .. } => true,
        }
    }

    fn plan(
        &self,
        track: &MusicTrack,
        tracks: &HashMap<String, MusicTrack>,
        ctx: &dyn ConditionContext,
    ) -> Result<PlayPlan, SchedulerError> {
        match &track.kind {
            MusicTrackKind::Single { file_key, .. } => Ok(PlayPlan::Single {
                file_key: file_key.clone(),
            }),
            MusicTrackKind::Silent { duration } => Ok(PlayPlan::Silent { seconds: *duration }),
            MusicTrackKind::Palette {
                layers,
                duration,
                fade_out,
            } => {
                let mut playlists = Vec::new();
                for layer in layers {
                    let steps = self.layer_steps(layer, tracks, ctx)?;
                    if steps
                        .iter()
                        .any(|step| matches!(step, LayerStep::Sound { .. }))
                    {
                        playlists.push(steps);
                    }
                }
                if playlists.is_empty() {
                    return Ok(PlayPlan::Silent { seconds: *duration });
                }
                Ok(PlayPlan::Palette {
                    layers: playlists,
                    duration: *duration,
                    fade_out: *fade_out,
                })
            }
        }
    }

    fn layer_steps(
        &self,
        layer: &[String],
        tracks: &HashMap<String, MusicTrack>,
        ctx: &dyn ConditionContext,
    ) -> Result<Vec<LayerStep>, SchedulerError> {
        let mut steps = Vec::new();
        for key in layer {
            let track = tracks
                .get(key)
                .ok_or_else(|| SchedulerError::UnknownTrack(key.clone()))?;
            let step = match &track.kind {
                MusicTrackKind::Single { file_key, .. } => LayerStep::Sound {
                    file_key: file_key.clone(),
                },
                MusicTrackKind::Silent { duration } => LayerStep::Gap { seconds: *duration },
                MusicTrackKind::Palette { .. } => {
                    return Err(SchedulerError::NestedPalette(key.clone()));
                }
            };
            if self.file_present(track) && evaluate(&track.conditions, ctx)? {
                steps.push(step);
            }
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::music::{Comparand, CompareOp};
    use crate::audio::conditions::FreshWorldContext;
    use crate::audio::test_support::catalog::{
        music_type, palette as catalog_palette, silent, single, track_map,
    };
    use crate::audio::test_support::condition::{condition, day, night};
    use crate::random::SplitMix64;

    fn clock(hour: f32) -> FreshWorldContext {
        FreshWorldContext::new(hour, [])
    }

    fn file(plan: Option<PlayPlan>) -> String {
        match plan {
            Some(PlayPlan::Single { file_key }) => file_key,
            other => panic!("expected a single track, got {other:?}"),
        }
    }

    /// Replays fixed answers so a test can steer the pick exactly.
    struct ScriptedRng(Vec<usize>);

    impl RandomSource for ScriptedRng {
        fn below(&mut self, n: usize) -> usize {
            let answer = self.0.remove(0);
            assert!(answer < n, "scripted {answer} is out of range 0..{n}");
            answer
        }
    }

    #[test]
    fn duplicate_entries_are_weighted_proportionally() {
        let tracks = track_map(vec![
            ("a", single("music/a.flac", vec![])),
            ("b", single("music/b.flac", vec![])),
        ]);
        let ty = music_type("type", 0, &["a", "a", "a", "b"]);
        let scheduler = TrackScheduler::default();
        let mut rng = SplitMix64::new(42);
        let ctx = clock(12.0);
        let draws = 4000;
        let a_count = (0..draws)
            .filter(|_| {
                file(scheduler.next_plan(&ty, &tracks, &ctx, &mut rng).unwrap()) == "music/a.flac"
            })
            .count();
        let share = a_count as f32 / draws as f32;
        assert!((0.72..0.78).contains(&share), "share of a was {share}");
    }

    #[test]
    fn seeded_generator_is_reproducible_and_in_range() {
        let draw = |seed| {
            let mut rng = SplitMix64::new(seed);
            (0..20).map(|_| rng.below(7)).collect::<Vec<_>>()
        };
        assert_eq!(draw(1), draw(1));
        assert_ne!(draw(1), draw(2));
        assert!(draw(1).iter().all(|&value| value < 7));
    }

    #[test]
    fn conditions_filter_day_and_night_tracks() {
        let tracks = track_map(vec![
            ("day", single("music/day.flac", day())),
            ("night", single("music/night.flac", night())),
        ]);
        let ty = music_type("type", 0, &["day", "night"]);
        let scheduler = TrackScheduler::default();
        for (hour, expected) in [
            (5.0, "music/night.flac"),
            (9.783, "music/day.flac"),
            (20.5, "music/night.flac"),
        ] {
            let plan = scheduler
                .next_plan(&ty, &tracks, &clock(hour), &mut SplitMix64::new(9))
                .unwrap();
            assert_eq!(file(plan), expected, "at {hour}");
        }
    }

    #[test]
    fn nothing_eligible_is_silence() {
        let tracks = track_map(vec![("night", single("music/night.flac", night()))]);
        let plan = TrackScheduler::default()
            .next_plan(
                &music_type("type", 0, &["night"]),
                &tracks,
                &clock(12.0),
                &mut SplitMix64::new(1),
            )
            .unwrap();
        assert_eq!(plan, None);
        let empty = TrackScheduler::default()
            .next_plan(
                &music_type("type", 0, &[]),
                &tracks,
                &clock(12.0),
                &mut SplitMix64::new(1),
            )
            .unwrap();
        assert_eq!(empty, None);
    }

    #[test]
    fn missing_files_are_excluded() {
        let tracks = track_map(vec![
            ("gone", single("music/gone.flac", vec![])),
            ("here", single("music/here.flac", vec![])),
        ]);
        let ty = music_type("type", 0, &["gone", "here"]);
        let scheduler = TrackScheduler::new(HashSet::from(["music/gone.flac".to_string()]));
        for seed in 0..20 {
            let plan = scheduler
                .next_plan(&ty, &tracks, &clock(12.0), &mut SplitMix64::new(seed))
                .unwrap();
            assert_eq!(file(plan), "music/here.flac");
        }
        let only_gone = scheduler
            .next_plan(
                &music_type("type", 0, &["gone"]),
                &tracks,
                &clock(12.0),
                &mut SplitMix64::new(0),
            )
            .unwrap();
        assert_eq!(only_gone, None);
    }

    #[test]
    fn silent_track_plans_its_seconds() {
        let tracks = track_map(vec![("pause", silent("pause", 30.0))]);
        let plan = TrackScheduler::default()
            .next_plan(
                &music_type("type", 0, &["pause"]),
                &tracks,
                &clock(12.0),
                &mut SplitMix64::new(1),
            )
            .unwrap();
        assert_eq!(plan, Some(PlayPlan::Silent { seconds: 30.0 }));
    }

    fn palette(layers: &[&[&str]]) -> MusicTrack {
        catalog_palette("p", layers, 120.0, Some(4.0))
    }

    fn sound(file_key: &str) -> LayerStep {
        LayerStep::Sound {
            file_key: file_key.to_string(),
        }
    }

    fn palette_plan(
        tracks: &HashMap<String, MusicTrack>,
        hour: f32,
        scheduler: &TrackScheduler,
    ) -> Result<Option<PlayPlan>, SchedulerError> {
        scheduler.next_plan(
            &music_type("type", 0, &["p"]),
            tracks,
            &clock(hour),
            &mut ScriptedRng(vec![0]),
        )
    }

    #[test]
    fn palette_layers_play_their_sub_tracks_in_order_with_silent_ones_as_gaps() {
        let tracks = track_map(vec![
            ("p", palette(&[&["l0a", "pause", "l0b"], &["l1"]])),
            ("l0a", single("music/l0a.flac", vec![])),
            ("pause", silent("pause", 5.0)),
            ("l0b", single("music/l0b.flac", vec![])),
            ("l1", single("music/l1.flac", vec![])),
        ]);

        let plan = palette_plan(&tracks, 12.0, &TrackScheduler::default()).unwrap();

        assert_eq!(
            plan,
            Some(PlayPlan::Palette {
                layers: vec![
                    vec![
                        sound("music/l0a.flac"),
                        LayerStep::Gap { seconds: 5.0 },
                        sound("music/l0b.flac"),
                    ],
                    vec![sound("music/l1.flac")],
                ],
                duration: 120.0,
                fade_out: Some(4.0),
            })
        );
    }

    #[test]
    fn palette_skips_ineligible_sub_tracks_and_drops_layers_without_sound() {
        let tracks = track_map(vec![
            (
                "p",
                palette(&[&["night_only", "day_only"], &["night_only"], &["hush"]]),
            ),
            ("night_only", single("music/night.flac", night())),
            ("day_only", single("music/day.flac", day())),
            ("hush", silent("pause", 5.0)),
        ]);

        let plan = palette_plan(&tracks, 12.0, &TrackScheduler::default()).unwrap();

        assert_eq!(
            plan,
            Some(PlayPlan::Palette {
                layers: vec![vec![sound("music/day.flac")]],
                duration: 120.0,
                fade_out: Some(4.0),
            })
        );
    }

    #[test]
    fn palette_skips_sub_tracks_whose_file_is_missing() {
        let tracks = track_map(vec![
            ("p", palette(&[&["gone", "here"]])),
            ("gone", single("music/gone.flac", vec![])),
            ("here", single("music/here.flac", vec![])),
        ]);
        let scheduler = TrackScheduler::new(HashSet::from(["music/gone.flac".to_string()]));

        let plan = palette_plan(&tracks, 12.0, &scheduler).unwrap();

        assert_eq!(
            plan,
            Some(PlayPlan::Palette {
                layers: vec![vec![sound("music/here.flac")]],
                duration: 120.0,
                fade_out: Some(4.0),
            })
        );
    }

    #[test]
    fn palette_without_any_sound_becomes_silence_of_its_duration() {
        let tracks = track_map(vec![
            ("p", palette(&[&["night_only"], &["hush"]])),
            ("night_only", single("music/night.flac", night())),
            ("hush", silent("pause", 5.0)),
        ]);

        let plan = palette_plan(&tracks, 12.0, &TrackScheduler::default()).unwrap();

        assert_eq!(plan, Some(PlayPlan::Silent { seconds: 120.0 }));
    }

    #[test]
    fn nested_palette_and_unknown_track_are_errors() {
        let tracks = track_map(vec![
            ("outer", palette(&[&["inner"]])),
            ("inner", palette(&[])),
        ]);
        let scheduler = TrackScheduler::default();
        assert_eq!(
            scheduler.next_plan(
                &music_type("type", 0, &["outer"]),
                &tracks,
                &clock(12.0),
                &mut ScriptedRng(vec![0]),
            ),
            Err(SchedulerError::NestedPalette("inner".to_string()))
        );
        assert_eq!(
            scheduler.next_plan(
                &music_type("type", 0, &["absent"]),
                &tracks,
                &clock(12.0),
                &mut ScriptedRng(vec![]),
            ),
            Err(SchedulerError::UnknownTrack("absent".to_string()))
        );
    }

    #[test]
    fn condition_errors_surface() {
        let tracks = track_map(vec![(
            "bad",
            single(
                "music/bad.flac",
                vec![condition(47, CompareOp::Equal, Comparand::Value(1.0))],
            ),
        )]);
        let result = TrackScheduler::default().next_plan(
            &music_type("type", 0, &["bad"]),
            &tracks,
            &clock(12.0),
            &mut ScriptedRng(vec![]),
        );
        assert_eq!(
            result,
            Err(SchedulerError::Condition(
                ConditionError::UnsupportedFunction(47)
            ))
        );
    }

    #[test]
    fn cycle_flag_is_bit_two() {
        let scheduler = TrackScheduler::default();
        assert!(scheduler.cycles(&music_type("type", FLAG_CYCLE_TRACKS, &[])));
        assert!(scheduler.cycles(&music_type("type", FLAG_CYCLE_TRACKS | 1, &[])));
        assert!(!scheduler.cycles(&music_type("type", FLAG_ABRUPT_TRANSITION | 1, &[])));
    }
}
