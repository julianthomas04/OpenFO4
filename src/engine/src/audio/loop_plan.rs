//! What the looping sounds have to do to follow the loops that are wanted. Pure: the loops that
//! play and the loops that are wanted are inputs, nothing is started here.

use std::collections::BTreeMap;

use kira::Decibels;

use super::decibels::amplitude_to_db;

/// A loop at or below this gain is not worth mixing; it counts as not wanted. kira takes this gain
/// and every lower one for silence.
pub const SILENT_DB: f32 = Decibels::SILENCE.0;
/// How far the gain of a playing loop has to be from its wanted gain before the loop is told.
pub const GAIN_STEP_DB: f32 = 0.5;
/// The amplitude of a file played as it is.
const UNITY_AMPLITUDE: f32 = 1.0;

/// A looping file, the gain it should have and the pitch it plays at.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopTarget {
    pub file_key: String,
    pub gain_db: f32,
    /// How much faster than its file the loop plays, `1` being as it is.
    pub playback_rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoopAction {
    Start {
        file_key: String,
        gain_db: f32,
        playback_rate: f64,
    },
    SetGain {
        file_key: String,
        gain_db: f32,
    },
    Stop {
        file_key: String,
    },
}

/// What the targets of one file ask for together.
#[derive(Debug, Clone, Copy)]
struct Wanted {
    gain_db: f32,
    playback_rate: f64,
}

/// The actions that bring the playing loops to the wanted ones: `active` holds the file key and the
/// gain each loop was last given. A wanted loop that is not playing starts, a playing loop that is
/// no longer wanted stops, and a playing loop whose gain is a step or more off follows it.
///
/// A target at or below `SILENT_DB`, or without a gain that is a number, is not wanted. A file
/// named by several targets plays once, at the pitch of the first of them and at the sum of their
/// amplitudes: at most unity, unless one of them is louder than that alone. Stops come first, then
/// the starts and gain changes; each group is in file key order.
pub fn plan_loops(active: &BTreeMap<String, f32>, targets: &[LoopTarget]) -> Vec<LoopAction> {
    let wanted = merge_targets(targets);
    let stops = active
        .keys()
        .filter(|file_key| !wanted.contains_key(*file_key))
        .map(|file_key| LoopAction::Stop {
            file_key: file_key.clone(),
        });
    let changes = wanted
        .iter()
        .filter_map(|(file_key, wanted)| match active.get(*file_key) {
            None => Some(LoopAction::Start {
                file_key: (*file_key).clone(),
                gain_db: wanted.gain_db,
                playback_rate: wanted.playback_rate,
            }),
            Some(&current) if (wanted.gain_db - current).abs() >= GAIN_STEP_DB => {
                Some(LoopAction::SetGain {
                    file_key: (*file_key).clone(),
                    gain_db: wanted.gain_db,
                })
            }
            Some(_) => None,
        });
    stops.chain(changes).collect()
}

fn merge_targets(targets: &[LoopTarget]) -> BTreeMap<&String, Wanted> {
    let mut by_file: BTreeMap<&String, Vec<&LoopTarget>> = BTreeMap::new();
    for target in targets.iter().filter(|target| target.gain_db > SILENT_DB) {
        by_file.entry(&target.file_key).or_default().push(target);
    }
    by_file
        .into_iter()
        .map(|(file_key, group)| {
            let gains: Vec<f32> = group.iter().map(|target| target.gain_db).collect();
            let wanted = Wanted {
                gain_db: summed_gain_db(&gains),
                playback_rate: group[0].playback_rate,
            };
            (file_key, wanted)
        })
        .collect()
}

/// The gain of several sounds of one file played as one: their amplitudes add up, to at most unity
/// or the loudest of them alone, so that no sound makes the file quieter.
fn summed_gain_db(gains: &[f32]) -> f32 {
    if let [only] = gains {
        return *only;
    }
    let amplitudes: Vec<f32> = gains
        .iter()
        .map(|&gain_db| Decibels(gain_db).as_amplitude())
        .collect();
    let loudest = amplitudes.iter().copied().fold(0.0, f32::max);
    let sum: f32 = amplitudes.iter().sum();
    amplitude_to_db(sum.min(loudest.max(UNITY_AMPLITUDE)))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const AS_IS: f64 = 1.0;
    const GAIN_TOLERANCE: f32 = 1e-3;

    fn target(file_key: &str, gain_db: f32) -> LoopTarget {
        pitched(file_key, gain_db, AS_IS)
    }

    fn pitched(file_key: &str, gain_db: f32, playback_rate: f64) -> LoopTarget {
        LoopTarget {
            file_key: file_key.to_string(),
            gain_db,
            playback_rate,
        }
    }

    fn active(loops: &[(&str, f32)]) -> BTreeMap<String, f32> {
        loops
            .iter()
            .map(|&(file_key, gain_db)| (file_key.to_string(), gain_db))
            .collect()
    }

    fn start(file_key: &str, gain_db: f32) -> LoopAction {
        start_pitched(file_key, gain_db, AS_IS)
    }

    fn start_pitched(file_key: &str, gain_db: f32, playback_rate: f64) -> LoopAction {
        LoopAction::Start {
            file_key: file_key.to_string(),
            gain_db,
            playback_rate,
        }
    }

    /// The gain and pitch of the one loop `targets` start.
    fn started(targets: &[LoopTarget]) -> (f32, f64) {
        let actions = plan_loops(&active(&[]), targets);
        match actions.as_slice() {
            [
                LoopAction::Start {
                    gain_db,
                    playback_rate,
                    ..
                },
            ] => (*gain_db, *playback_rate),
            other => panic!("expected one start, got {other:?}"),
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < GAIN_TOLERANCE,
            "{actual} dB is not {expected} dB"
        );
    }

    fn set_gain(file_key: &str, gain_db: f32) -> LoopAction {
        LoopAction::SetGain {
            file_key: file_key.to_string(),
            gain_db,
        }
    }

    fn stop(file_key: &str) -> LoopAction {
        LoopAction::Stop {
            file_key: file_key.to_string(),
        }
    }

    #[test]
    fn a_wanted_loop_that_is_not_playing_starts_at_its_gain() {
        assert_eq!(
            plan_loops(&active(&[]), &[target("wind", -6.0)]),
            [start("wind", -6.0)]
        );
    }

    #[test]
    fn a_playing_loop_that_is_no_longer_wanted_stops() {
        assert_eq!(plan_loops(&active(&[("rain", -3.0)]), &[]), [stop("rain")]);
    }

    #[test]
    fn a_loop_that_is_playing_and_wanted_at_the_same_gain_needs_nothing() {
        assert_eq!(
            plan_loops(&active(&[("wind", -6.0)]), &[target("wind", -6.0)]),
            []
        );
    }

    #[test]
    fn a_playing_loop_follows_a_gain_that_moved_a_step_and_ignores_a_smaller_move() {
        let playing = active(&[("wind", -6.0)]);

        assert_eq!(
            plan_loops(&playing, &[target("wind", -6.0 + GAIN_STEP_DB)]),
            [set_gain("wind", -6.0 + GAIN_STEP_DB)]
        );
        assert_eq!(
            plan_loops(&playing, &[target("wind", -6.0 - GAIN_STEP_DB)]),
            [set_gain("wind", -6.0 - GAIN_STEP_DB)]
        );
        assert_eq!(
            plan_loops(&playing, &[target("wind", -6.0 + GAIN_STEP_DB * 0.9)]),
            []
        );
    }

    #[test]
    fn a_silent_target_stops_a_playing_loop_and_starts_no_loop() {
        for silent in [SILENT_DB, -80.0, f32::NEG_INFINITY, f32::NAN] {
            assert_eq!(
                plan_loops(&active(&[("rain", -3.0)]), &[target("rain", silent)]),
                [stop("rain")],
                "{silent}"
            );
            assert_eq!(
                plan_loops(&active(&[]), &[target("rain", silent)]),
                [],
                "{silent}"
            );
        }
    }

    #[test]
    fn a_target_just_above_silence_is_wanted() {
        let barely_audible = SILENT_DB + 0.1;

        assert_eq!(
            plan_loops(&active(&[]), &[target("wind", barely_audible)]),
            [start("wind", barely_audible)]
        );
    }

    #[test]
    fn a_loop_starts_at_the_pitch_of_its_target() {
        assert_eq!(
            plan_loops(&active(&[]), &[pitched("wind", -6.0, 1.25)]),
            [start_pitched("wind", -6.0, 1.25)]
        );
    }

    #[test]
    fn a_file_named_by_several_targets_plays_once_at_the_sum_of_their_amplitudes() {
        let half = 20.0 * 0.5_f32.log10();
        let quarter = 20.0 * 0.25_f32.log10();
        let targets = [target("wind", half), target("wind", quarter)];

        let actions = plan_loops(&active(&[]), &targets);

        assert_eq!(actions.len(), 1, "{actions:?}");
        let (gain_db, _) = started(&targets);
        assert_close(gain_db, 20.0 * 0.75_f32.log10());
    }

    #[test]
    fn two_targets_of_one_loudness_make_the_file_six_db_louder() {
        let targets = [target("wind", -14.0), target("wind", -14.0)];

        let (gain_db, _) = started(&targets);

        assert_close(gain_db, -14.0 + 6.0206);
    }

    #[test]
    fn the_sum_of_a_file_stops_at_unity() {
        let half = 20.0 * 0.5_f32.log10();
        let targets = [
            target("wind", half),
            target("wind", half),
            target("wind", half),
        ];

        let (gain_db, _) = started(&targets);

        assert_close(gain_db, 0.0);
    }

    #[test]
    fn a_target_louder_than_unity_is_never_made_quieter_by_another() {
        let alone = started(&[target("wind", 3.0)]).0;
        let with_others = started(&[target("wind", 3.0), target("wind", -30.0)]).0;

        assert_close(alone, 3.0);
        assert_close(with_others, 3.0);
    }

    #[test]
    fn a_file_named_by_several_targets_plays_at_the_pitch_of_the_first() {
        let targets = [
            pitched("wind", -12.0, 1.1),
            pitched("wind", -12.0, 0.9),
            pitched("wind", -12.0, 1.0),
        ];

        let (_, playback_rate) = started(&targets);

        assert!((playback_rate - 1.1).abs() < 1e-12);
    }

    #[test]
    fn a_target_that_is_silent_or_not_a_number_adds_nothing_to_its_file() {
        let targets = [
            target("wind", -90.0),
            target("wind", -9.0),
            target("wind", f32::NAN),
            target("wind", SILENT_DB),
        ];

        assert_eq!(plan_loops(&active(&[]), &targets), [start("wind", -9.0)]);
    }

    #[test]
    fn a_silent_first_target_does_not_decide_the_pitch_of_its_file() {
        let targets = [pitched("wind", -90.0, 2.0), pitched("wind", -9.0, 1.5)];

        assert_eq!(
            plan_loops(&active(&[]), &targets),
            [start_pitched("wind", -9.0, 1.5)]
        );
    }

    #[test]
    fn stops_come_first_and_each_group_is_in_file_key_order() {
        let playing = active(&[("b_old", -3.0), ("a_old", -3.0), ("d_kept", -6.0)]);
        let targets = [
            target("z_new", -6.0),
            target("d_kept", -12.0),
            target("c_new", -6.0),
        ];

        assert_eq!(
            plan_loops(&playing, &targets),
            [
                stop("a_old"),
                stop("b_old"),
                start("c_new", -6.0),
                set_gain("d_kept", -12.0),
                start("z_new", -6.0),
            ]
        );
    }

    fn any_gain() -> impl Strategy<Value = f32> {
        prop_oneof![
            Just(f32::NAN),
            Just(f32::NEG_INFINITY),
            Just(SILENT_DB),
            -90.0_f32..6.0,
        ]
    }

    fn any_file() -> impl Strategy<Value = String> {
        "[a-d]"
    }

    fn apply(playing: &mut BTreeMap<String, f32>, actions: Vec<LoopAction>) {
        for action in actions {
            match action {
                LoopAction::Start {
                    file_key, gain_db, ..
                }
                | LoopAction::SetGain { file_key, gain_db } => {
                    playing.insert(file_key, gain_db);
                }
                LoopAction::Stop { file_key } => {
                    playing.remove(&file_key);
                }
            }
        }
    }

    proptest! {
        #[test]
        fn following_the_plan_leaves_exactly_the_audible_targets_playing_and_nothing_to_do(
            playing in proptest::collection::btree_map(any_file(), -59.0_f32..6.0, 0..4),
            targets in proptest::collection::vec((any_file(), any_gain()), 0..6),
        ) {
            let targets: Vec<LoopTarget> = targets
                .into_iter()
                .map(|(file_key, gain_db)| pitched(&file_key, gain_db, AS_IS))
                .collect();
            let mut playing = playing;

            let actions = plan_loops(&playing, &targets);
            apply(&mut playing, actions);

            let audible: std::collections::BTreeSet<&String> = targets
                .iter()
                .filter(|target| target.gain_db > SILENT_DB)
                .map(|target| &target.file_key)
                .collect();
            prop_assert_eq!(playing.keys().collect::<std::collections::BTreeSet<_>>(), audible);
            prop_assert_eq!(plan_loops(&playing, &targets), vec![]);
        }

        #[test]
        fn a_file_named_by_several_targets_plays_between_its_loudest_target_and_unity_or_that_target(
            gains in proptest::collection::vec(-59.0_f32..6.0, 1..6),
        ) {
            let targets: Vec<LoopTarget> =
                gains.iter().map(|&gain_db| target("wind", gain_db)).collect();
            let loudest = gains.iter().copied().fold(f32::MIN, f32::max);

            let (gain_db, _) = started(&targets);

            prop_assert!(gain_db >= loudest - GAIN_TOLERANCE, "{gain_db} under {loudest}");
            prop_assert!(
                gain_db <= loudest.max(0.0) + GAIN_TOLERANCE,
                "{gain_db} over {loudest} and unity"
            );
        }
    }
}
