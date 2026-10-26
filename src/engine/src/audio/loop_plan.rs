//! What the looping sounds have to do to follow the loops that are wanted. Pure: the loops that
//! play and the loops that are wanted are inputs, nothing is started here.

use std::collections::BTreeMap;

use kira::Decibels;

/// A loop at or below this gain is not worth mixing; it counts as not wanted. kira takes this gain
/// and every lower one for silence.
pub const SILENT_DB: f32 = Decibels::SILENCE.0;
/// How far the gain of a playing loop has to be from its wanted gain before the loop is told.
pub const GAIN_STEP_DB: f32 = 0.5;

/// A looping file and the gain it should have.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopTarget {
    pub file_key: String,
    pub gain_db: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoopAction {
    Start { file_key: String, gain_db: f32 },
    SetGain { file_key: String, gain_db: f32 },
    Stop { file_key: String },
}

/// The actions that bring the playing loops to the wanted ones: `active` holds the file key and the
/// gain each loop was last given. A wanted loop that is not playing starts, a playing loop that is
/// no longer wanted stops, and a playing loop whose gain is a step or more off follows it.
///
/// A target at or below `SILENT_DB`, or without a gain that is a number, is not wanted. A file
/// named by several targets takes the loudest of them. Stops come first, then the starts and gain
/// changes; each group is in file key order.
pub fn plan_loops(active: &BTreeMap<String, f32>, targets: &[LoopTarget]) -> Vec<LoopAction> {
    let wanted = audible_gains(targets);
    let stops = active
        .keys()
        .filter(|file_key| !wanted.contains_key(*file_key))
        .map(|file_key| LoopAction::Stop {
            file_key: file_key.clone(),
        });
    let changes = wanted
        .iter()
        .filter_map(|(file_key, &gain_db)| match active.get(*file_key) {
            None => Some(LoopAction::Start {
                file_key: (*file_key).clone(),
                gain_db,
            }),
            Some(&current) if (gain_db - current).abs() >= GAIN_STEP_DB => {
                Some(LoopAction::SetGain {
                    file_key: (*file_key).clone(),
                    gain_db,
                })
            }
            Some(_) => None,
        });
    stops.chain(changes).collect()
}

fn audible_gains(targets: &[LoopTarget]) -> BTreeMap<&String, f32> {
    let mut gains = BTreeMap::new();
    for target in targets.iter().filter(|target| target.gain_db > SILENT_DB) {
        gains
            .entry(&target.file_key)
            .and_modify(|gain: &mut f32| *gain = gain.max(target.gain_db))
            .or_insert(target.gain_db);
    }
    gains
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn target(file_key: &str, gain_db: f32) -> LoopTarget {
        LoopTarget {
            file_key: file_key.to_string(),
            gain_db,
        }
    }

    fn active(loops: &[(&str, f32)]) -> BTreeMap<String, f32> {
        loops
            .iter()
            .map(|&(file_key, gain_db)| (file_key.to_string(), gain_db))
            .collect()
    }

    fn start(file_key: &str, gain_db: f32) -> LoopAction {
        LoopAction::Start {
            file_key: file_key.to_string(),
            gain_db,
        }
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
    fn a_file_named_by_several_targets_takes_the_loudest_gain() {
        let targets = [
            target("wind", -12.0),
            target("wind", -3.0),
            target("wind", f32::NAN),
            target("wind", -20.0),
        ];

        assert_eq!(plan_loops(&active(&[]), &targets), [start("wind", -3.0)]);
    }

    #[test]
    fn a_silent_duplicate_does_not_hide_an_audible_one() {
        let targets = [target("wind", -90.0), target("wind", -9.0)];

        assert_eq!(plan_loops(&active(&[]), &targets), [start("wind", -9.0)]);
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
                LoopAction::Start { file_key, gain_db }
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
                .map(|(file_key, gain_db)| LoopTarget { file_key, gain_db })
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
    }
}
