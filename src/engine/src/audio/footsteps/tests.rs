use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use super::resolve::COMPOUND_DESCRIPTOR;
use super::*;
use crate::assets::db::footsteps::FootstepCatalog;
use crate::assets::db::footsteps::STANDARD_DESCRIPTOR_TYPE;
use crate::audio::conditions::{ConditionContext, FreshWorldContext};
use crate::audio::test_support::footsteps::{
    DIRT, DIRT_FIRST_PERSON_FILES, DIRT_QUIET_FILE, GRASS, ORPHAN, SILENT_RUNNING_FILE, SNEAK_FILE,
    SPRINT_FILE, STONE, STONE_FILE, THIRD_PERSON_FILE, UNKNOWN, all_files, catalog,
    expected_gain_db,
};
use crate::audio::test_support::{
    HALF_SECOND_SAMPLES, MockAudio, PeakReader, SAMPLE_RATE, SEED, SharedBuffer,
    mock_manager_settings, render, render_buffer, sine_flac, undecodable_flac,
};
use crate::audio::{AudioError, AudioFrame, AudioSystem, MenuMusicSettings};
use crate::footsteps::{FootTag, FootstepEvent, Gait, SurfaceMaterial};
use crate::game_flow::Phase;

const GAIN_TOLERANCE: f32 = 1e-4;

fn event(gait: Gait, tag: FootTag) -> FootstepEvent {
    FootstepEvent {
        tag,
        gait,
        surface: SurfaceMaterial::None,
    }
}

fn run_right() -> FootstepEvent {
    event(Gait::Run, FootTag::FootRight)
}

fn world() -> FreshWorldContext {
    FreshWorldContext::new(0.0, [])
}

fn resolved(
    catalog: &FootstepCatalog,
    event: FootstepEvent,
    material: Option<u32>,
) -> Result<Resolution, ResolveError> {
    resolve(
        catalog,
        &world(),
        event,
        material,
        &HashSet::new(),
        &mut SplitMix64::new(SEED),
    )
}

fn layers(catalog: &FootstepCatalog, event: FootstepEvent, material: u32) -> Vec<LayerPlay> {
    match resolved(catalog, event, Some(material)).unwrap() {
        Resolution::Play(layers) => layers,
        Resolution::Silent(reason) => panic!("expected sound, got silence: {reason}"),
    }
}

fn silence(catalog: &FootstepCatalog, event: FootstepEvent, material: Option<u32>) -> SilentReason {
    match resolved(catalog, event, material).unwrap() {
        Resolution::Silent(reason) => reason,
        Resolution::Play(layers) => panic!("expected silence, got {layers:?}"),
    }
}

fn files(layers: &[LayerPlay]) -> BTreeSet<&str> {
    layers.iter().map(|layer| layer.file_key.as_str()).collect()
}

fn modify_descriptor(
    catalog: &mut FootstepCatalog,
    key: &str,
    change: impl FnOnce(&mut crate::assets::db::footsteps::SoundDescriptor),
) {
    change(catalog.sound_descriptors.get_mut(key).unwrap());
}

mod tag_selection {
    use super::*;

    #[test]
    fn each_gait_plays_the_sound_of_its_own_footstep_set_entry() {
        let catalog = catalog();
        assert_eq!(
            files(&layers(
                &catalog,
                event(Gait::Sprint, FootTag::FootSprintRight),
                DIRT
            )),
            BTreeSet::from([SPRINT_FILE])
        );
        assert_eq!(
            files(&layers(
                &catalog,
                event(Gait::Sneak, FootTag::FootLeft),
                DIRT
            )),
            BTreeSet::from([SNEAK_FILE])
        );
        assert_eq!(
            files(&layers(
                &catalog,
                event(Gait::Run, FootTag::FootLeft),
                STONE
            )),
            BTreeSet::from([STONE_FILE])
        );
    }

    #[test]
    fn the_tag_picks_the_footstep_within_the_gait() {
        let mut catalog = catalog();
        catalog.footsteps.get_mut("run_l").unwrap().impact_data_set =
            Some("sneak_data".to_string());
        let left = layers(&catalog, event(Gait::Run, FootTag::FootLeft), DIRT);
        let right = layers(&catalog, run_right(), DIRT);
        assert_eq!(files(&left), BTreeSet::from([SNEAK_FILE]));
        assert_ne!(files(&right), files(&left));
    }

    #[test]
    fn a_tag_or_gait_the_set_lacks_is_silent() {
        let catalog = catalog();
        for missing in [
            event(Gait::Sprint, FootTag::FootRight),
            event(Gait::Run, FootTag::JumpUp),
            event(Gait::Walk, FootTag::FootRight),
        ] {
            assert_eq!(
                silence(&catalog, missing, Some(DIRT)),
                SilentReason::NoFootstep {
                    gait: missing.gait,
                    tag: missing.tag
                }
            );
        }
    }

    #[test]
    fn a_footstep_without_an_impact_data_set_is_silent() {
        let mut catalog = catalog();
        catalog.footsteps.get_mut("run_r").unwrap().impact_data_set = None;
        assert_eq!(
            silence(&catalog, run_right(), Some(DIRT)),
            SilentReason::NoImpactDataSet {
                footstep: "run_r".to_string()
            }
        );
    }
}

mod material_walk {
    use super::*;

    #[test]
    fn a_listed_material_plays_its_own_impact() {
        let catalog = catalog();
        assert_eq!(
            files(&layers(&catalog, run_right(), STONE)),
            BTreeSet::from([STONE_FILE])
        );
    }

    #[test]
    fn an_unlisted_material_plays_the_impact_of_its_nearest_listed_parent() {
        let catalog = catalog();
        assert_eq!(
            files(&layers(&catalog, run_right(), GRASS)),
            files(&layers(&catalog, run_right(), DIRT))
        );
    }

    #[test]
    fn the_nearest_ancestor_wins_over_a_farther_one() {
        let mut catalog = catalog();
        catalog.material_parents.insert(0x70, Some(GRASS));
        catalog.material_parents.insert(GRASS, Some(STONE));
        assert_eq!(
            files(&layers(&catalog, run_right(), 0x70)),
            BTreeSet::from([STONE_FILE])
        );
    }

    #[test]
    fn a_chain_without_a_listed_material_is_a_dead_end() {
        let catalog = catalog();
        assert_eq!(
            silence(&catalog, run_right(), Some(ORPHAN)),
            SilentReason::UnlistedMaterial(ORPHAN)
        );
        assert_eq!(
            silence(&catalog, run_right(), Some(UNKNOWN)),
            SilentReason::UnlistedMaterial(UNKNOWN)
        );
    }

    #[test]
    fn a_material_listed_for_another_gait_only_is_a_dead_end() {
        let catalog = catalog();
        assert_eq!(
            silence(
                &catalog,
                event(Gait::Sprint, FootTag::FootSprintRight),
                Some(STONE)
            ),
            SilentReason::UnlistedMaterial(STONE)
        );
    }

    #[test]
    fn no_material_is_silent() {
        let catalog = catalog();
        assert_eq!(
            silence(&catalog, run_right(), None),
            SilentReason::NoMaterial
        );
    }

    #[test]
    fn a_parent_cycle_is_an_error() {
        let mut catalog = catalog();
        catalog.material_parents.insert(ORPHAN, Some(UNKNOWN));
        catalog.material_parents.insert(UNKNOWN, Some(ORPHAN));
        assert_eq!(
            resolved(&catalog, run_right(), Some(ORPHAN)),
            Err(ResolveError::MaterialCycle(ORPHAN))
        );
    }
}

struct Situation {
    first_person: bool,
    silent_running: bool,
}

impl ConditionContext for Situation {
    fn game_hour(&self) -> f32 {
        0.0
    }
    fn global(&self, _key: &str) -> Option<f32> {
        None
    }
    fn quest_running(&self, _quest: &str) -> bool {
        false
    }
    fn quest_stage(&self, _quest: &str) -> i32 {
        0
    }
    fn stage_done(&self, _quest: &str, _stage: i32) -> bool {
        false
    }
    fn vm_quest_variable(&self, _quest: &str, _variable: &str) -> f32 {
        0.0
    }
    fn is_first_person(&self) -> bool {
        self.first_person
    }
    fn has_silent_running_perk(&self) -> bool {
        self.silent_running
    }
}

mod compound_layers {
    use super::*;

    fn dirt_in(situation: &Situation) -> Resolution {
        resolve(
            &catalog(),
            situation,
            run_right(),
            Some(DIRT),
            &HashSet::new(),
            &mut SplitMix64::new(SEED),
        )
        .unwrap()
    }

    fn played_files(resolution: &Resolution) -> BTreeSet<String> {
        match resolution {
            Resolution::Play(layers) => layers.iter().map(|layer| layer.file_key.clone()).collect(),
            Resolution::Silent(reason) => panic!("expected sound, got silence: {reason}"),
        }
    }

    #[test]
    fn the_fresh_world_plays_the_first_person_and_the_perkless_layers_only() {
        let catalog = catalog();
        let played = layers(&catalog, run_right(), DIRT);
        assert_eq!(played.len(), 2);
        let played = files(&played);
        assert!(
            DIRT_FIRST_PERSON_FILES
                .iter()
                .any(|file| played.contains(file))
        );
        assert!(played.contains(DIRT_QUIET_FILE));
        assert!(!played.contains(THIRD_PERSON_FILE), "391 == 0 is silent");
        assert!(!played.contains(SILENT_RUNNING_FILE), "448 == 1 is silent");
    }

    #[test]
    fn the_silent_running_perk_swaps_the_layers_that_need_it() {
        let played = played_files(&dirt_in(&Situation {
            first_person: true,
            silent_running: true,
        }));
        assert!(played.contains(SILENT_RUNNING_FILE));
        assert!(!played.contains(DIRT_QUIET_FILE));
    }

    #[test]
    fn third_person_swaps_the_first_person_layer_for_the_third_person_one() {
        let played = played_files(&dirt_in(&Situation {
            first_person: false,
            silent_running: false,
        }));
        assert!(played.contains(THIRD_PERSON_FILE));
        assert!(
            !played
                .iter()
                .any(|file| DIRT_FIRST_PERSON_FILES.contains(&file.as_str()))
        );
    }

    #[test]
    fn every_layer_failing_its_conditions_is_silent() {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, "dirt_quiet", |descriptor| {
            descriptor.conditions = crate::audio::test_support::footsteps::silent_running(1.0);
        });
        modify_descriptor(&mut catalog, "dirt_1p", |descriptor| {
            descriptor.conditions = crate::audio::test_support::footsteps::first_person(0.0);
        });
        assert_eq!(
            silence(&catalog, run_right(), Some(DIRT)),
            SilentReason::NoLayerPlays {
                descriptor: "dirt_sound".to_string()
            }
        );
    }

    #[test]
    fn a_compound_sound_nested_in_a_compound_plays_its_layers() {
        let mut catalog = catalog();
        catalog.sound_descriptors.insert(
            "outer".to_string(),
            crate::audio::test_support::footsteps::compound("outer", &["dirt_sound"]),
        );
        catalog.impacts.get_mut("dirt_impact").unwrap().player_sound = Some("outer".to_string());
        assert_eq!(layers(&catalog, run_right(), DIRT).len(), 2);
    }

    #[test]
    fn a_standard_sound_that_fails_its_own_conditions_is_silent() {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, "stone_sound", |descriptor| {
            descriptor.conditions = crate::audio::test_support::footsteps::silent_running(1.0);
        });
        assert_eq!(
            silence(&catalog, run_right(), Some(STONE)),
            SilentReason::NoLayerPlays {
                descriptor: "stone_sound".to_string()
            }
        );
    }

    #[test]
    fn an_impact_without_a_player_sound_is_silent() {
        let mut catalog = catalog();
        catalog
            .impacts
            .get_mut("stone_impact")
            .unwrap()
            .player_sound = None;
        assert_eq!(
            silence(&catalog, run_right(), Some(STONE)),
            SilentReason::NoPlayerSound {
                impact: "stone_impact".to_string()
            }
        );
    }
}

mod layer_values {
    use super::*;

    const DRAWS: usize = 400;

    fn varied_catalog() -> FootstepCatalog {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, "stone_sound", |descriptor| {
            descriptor.freq_shift_pct = 5;
            descriptor.freq_variance_pct = 10;
            descriptor.db_variance = 3;
            descriptor.sound_files = vec!["Sound\\a.wav".to_string(), "Sound\\b.wav".to_string()];
        });
        catalog
    }

    fn draws() -> Vec<LayerPlay> {
        let catalog = varied_catalog();
        let mut rng = SplitMix64::new(SEED);
        (0..DRAWS)
            .flat_map(|_| {
                match resolve(
                    &catalog,
                    &world(),
                    run_right(),
                    Some(STONE),
                    &HashSet::new(),
                    &mut rng,
                )
                .unwrap()
                {
                    Resolution::Play(layers) => layers,
                    Resolution::Silent(reason) => panic!("silent: {reason}"),
                }
            })
            .collect()
    }

    #[test]
    fn the_playback_rate_stays_within_shift_plus_minus_variance_and_uses_the_range() {
        let rates: Vec<f64> = draws()
            .iter()
            .map(|layer| layer.mix.playback_rate)
            .collect();
        assert!(rates.iter().all(|rate| (0.95..1.15).contains(rate)));
        assert!(rates.iter().any(|&rate| rate < 0.97));
        assert!(rates.iter().any(|&rate| rate > 1.13));
    }

    #[test]
    fn the_gain_stays_within_the_db_variance_of_the_chain_total_and_uses_the_range() {
        let base = expected_gain_db();
        let gains: Vec<f32> = draws().iter().map(|layer| layer.mix.gain_db).collect();
        assert!(gains.iter().all(|gain| (gain - base).abs() < 3.0));
        assert!(gains.iter().any(|&gain| gain < base - 2.5));
        assert!(gains.iter().any(|&gain| gain > base + 2.5));
    }

    #[test]
    fn the_file_is_chosen_uniformly_from_the_descriptors_files() {
        let draws = draws();
        let first = draws
            .iter()
            .filter(|layer| layer.file_key == "sound/a.flac")
            .count();
        let second = draws
            .iter()
            .filter(|layer| layer.file_key == "sound/b.flac")
            .count();
        assert_eq!(first + second, DRAWS);
        assert!(
            first.abs_diff(second) < DRAWS / 5,
            "{first} against {second}"
        );
    }

    #[test]
    fn without_variance_every_layer_has_the_chain_total_and_the_plain_rate() {
        let catalog = catalog();
        for layer in layers(&catalog, run_right(), DIRT) {
            assert!((layer.mix.playback_rate - 1.0).abs() < 1e-12);
            assert!((layer.mix.gain_db - expected_gain_db()).abs() < GAIN_TOLERANCE);
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_layers() {
        assert_eq!(draws(), draws());
    }
}

mod data_errors {
    use super::*;

    fn stone_error(change: impl FnOnce(&mut FootstepCatalog)) -> ResolveError {
        let mut catalog = catalog();
        change(&mut catalog);
        resolved(&catalog, run_right(), Some(STONE)).unwrap_err()
    }

    #[test]
    fn a_looping_footstep_sound_is_an_error() {
        let error = stone_error(|catalog| {
            modify_descriptor(catalog, "stone_sound", |d| d.looping_flags = 0x0800);
        });
        assert_eq!(error, ResolveError::Looping("stone_sound".to_string()));
    }

    #[test]
    fn a_sound_without_files_is_an_error() {
        let error = stone_error(|catalog| {
            modify_descriptor(catalog, "stone_sound", |d| d.sound_files.clear());
        });
        assert_eq!(error, ResolveError::NoSoundFiles("stone_sound".to_string()));
    }

    #[test]
    fn a_sound_without_an_output_model_is_an_error() {
        let error = stone_error(|catalog| {
            modify_descriptor(catalog, "stone_sound", |d| d.output_model = None);
        });
        assert_eq!(
            error,
            ResolveError::NoOutputModel("stone_sound".to_string())
        );
    }

    #[test]
    fn an_unsupported_descriptor_type_is_an_error() {
        let error = stone_error(|catalog| {
            modify_descriptor(catalog, "stone_sound", |d| d.descriptor_type = 0xED15_7AE3);
        });
        assert_eq!(
            error,
            ResolveError::UnsupportedDescriptor {
                descriptor: "stone_sound".to_string(),
                descriptor_type: 0xED15_7AE3
            }
        );
    }

    #[test]
    fn a_compound_that_contains_itself_is_an_error() {
        let error = stone_error(|catalog| {
            modify_descriptor(catalog, "stone_sound", |d| {
                d.descriptor_type = COMPOUND_DESCRIPTOR;
                d.children = vec!["stone_sound".to_string()];
            });
        });
        assert_eq!(
            error,
            ResolveError::DescriptorCycle("stone_sound".to_string())
        );
    }

    #[test]
    fn a_missing_record_is_an_error_naming_it() {
        let error = stone_error(|catalog| {
            catalog.sound_descriptors.remove("stone_sound");
        });
        assert_eq!(
            error,
            ResolveError::UnknownRecord {
                kind: "sound descriptor",
                key: "stone_sound".to_string()
            }
        );
    }

    #[test]
    fn unusable_mix_data_is_an_error_naming_the_sound() {
        let error = stone_error(|catalog| {
            catalog.sound_categories.clear();
        });
        assert!(
            matches!(&error, ResolveError::Mix { descriptor, source: MixError::UnknownCategory(_) } if descriptor == "stone_sound"),
            "{error:?}"
        );
    }

    #[test]
    fn a_standard_descriptor_keeps_its_own_type_constant() {
        assert_eq!(
            catalog().sound_descriptors["stone_sound"].descriptor_type,
            STANDARD_DESCRIPTOR_TYPE
        );
    }
}

fn write_file(root: &Path, key: &str, bytes: &[u8]) {
    let path = root.join("audio").join(key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn sine() -> Vec<u8> {
    sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES)
}

fn install() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for file in all_files() {
        write_file(root.path(), file, &sine());
    }
    root
}

mod startup_and_logging {
    use super::*;

    fn footsteps(root: &Path) -> Result<Footsteps, FootstepError> {
        Footsteps::new(catalog(), &root.join("audio"), SEED)
    }

    #[test]
    fn a_complete_install_opens() {
        assert!(footsteps(install().path()).is_ok());
    }

    #[test]
    fn a_missing_audio_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            footsteps(root.path()),
            Err(FootstepError::AudioDirectoryMissing(_))
        ));
    }

    #[test]
    fn a_missing_file_is_reported_once_even_when_its_layer_is_silenced_by_conditions() {
        for file in [THIRD_PERSON_FILE, SILENT_RUNNING_FILE] {
            let root = install();
            std::fs::remove_file(root.path().join("audio").join(file)).unwrap();
            let log = SharedBuffer::default();
            let _guard = log.capture("openfo4=warn");
            let footsteps = footsteps(root.path()).unwrap();
            assert_eq!(
                footsteps.missing_files(),
                &HashSet::from([file.to_string()])
            );
            let logged = log.drain();
            let missing_lines: Vec<&str> = logged
                .lines()
                .filter(|line| line.contains("is missing"))
                .collect();
            assert_eq!(missing_lines.len(), 1, "{logged}");
            assert!(missing_lines[0].contains(file), "{logged}");
        }
    }

    #[test]
    fn a_missing_file_is_skipped_while_the_descriptors_other_files_still_play() {
        let root = install();
        let missing = DIRT_FIRST_PERSON_FILES[0];
        std::fs::remove_file(root.path().join("audio").join(missing)).unwrap();
        let mut footsteps = footsteps(root.path()).unwrap();
        for _ in 0..20 {
            let played: BTreeSet<String> = footsteps
                .step(run_right(), Some(DIRT))
                .unwrap()
                .into_iter()
                .map(|layer| layer.file_key)
                .collect();
            assert!(!played.contains(missing), "{played:?}");
            assert!(played.contains(DIRT_FIRST_PERSON_FILES[1]), "{played:?}");
            assert!(played.contains(DIRT_QUIET_FILE), "{played:?}");
        }
    }

    #[test]
    fn a_descriptor_left_without_files_is_silent_and_logged_once() {
        let root = install();
        std::fs::remove_file(root.path().join("audio").join(STONE_FILE)).unwrap();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let mut footsteps = footsteps(root.path()).unwrap();
        let startup = log.drain();
        assert_eq!(startup.lines().count(), 2, "{startup}");
        assert!(
            startup.contains("stone_sound has no file left"),
            "{startup}"
        );
        for _ in 0..3 {
            assert_eq!(footsteps.step(run_right(), Some(STONE)).unwrap(), []);
        }
        let steps = log.drain();
        assert_eq!(steps.lines().count(), 1, "{steps}");
    }

    #[test]
    fn a_file_missing_for_one_descriptor_leaves_the_other_descriptors_audible() {
        let root = install();
        std::fs::remove_file(root.path().join("audio").join(STONE_FILE)).unwrap();
        let mut footsteps = footsteps(root.path()).unwrap();
        assert_ne!(footsteps.step(run_right(), Some(DIRT)).unwrap(), []);
    }

    #[test]
    fn a_file_whose_header_cannot_be_opened_is_an_error() {
        let root = install();
        write_file(root.path(), STONE_FILE, &[]);
        assert!(matches!(
            footsteps(root.path()),
            Err(FootstepError::UnreadableFile { .. })
        ));
    }

    #[test]
    fn a_silent_step_is_logged_once_per_cause() {
        let root = install();
        let mut footsteps = footsteps(root.path()).unwrap();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4::audio::footsteps=warn");
        for _ in 0..3 {
            assert_eq!(footsteps.step(run_right(), Some(ORPHAN)).unwrap(), []);
        }
        let first = log.drain();
        assert_eq!(first.lines().count(), 1, "{first}");
        assert!(first.contains("0000000F"), "{first}");
        footsteps.step(run_right(), Some(UNKNOWN)).unwrap();
        footsteps.step(run_right(), None).unwrap();
        footsteps.step(run_right(), None).unwrap();
        assert_eq!(log.drain().lines().count(), 2);
    }

    #[test]
    fn a_step_with_sound_logs_nothing_and_returns_its_layers() {
        let root = install();
        let mut footsteps = footsteps(root.path()).unwrap();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4::audio::footsteps=warn");
        assert_eq!(footsteps.step(run_right(), Some(STONE)).unwrap().len(), 1);
        assert_eq!(log.drain(), "");
    }
}

struct Rig {
    root: tempfile::TempDir,
    audio: MockAudio,
    peak: PeakReader,
}

fn rig_with(root: tempfile::TempDir, catalog: FootstepCatalog) -> Rig {
    let (manager_settings, peak) = mock_manager_settings(SAMPLE_RATE);
    let settings = MenuMusicSettings {
        enabled: false,
        file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
        fade: Duration::from_millis(100),
        attenuation_millibels: 300,
    };
    let mut audio = AudioSystem::new(manager_settings, root.path(), settings).unwrap();
    let footsteps = Footsteps::new(catalog, &root.path().join("audio"), SEED).unwrap();
    audio.set_footsteps(footsteps);
    Rig { root, audio, peak }
}

fn rig() -> Rig {
    rig_with(install(), catalog())
}

fn enter(rig: &mut Rig, phase: Phase) {
    rig.audio.sync(AudioFrame::from(phase)).unwrap();
}

fn audible_after_playing(rig: &mut Rig, material: u32) -> f32 {
    rig.peak.take();
    rig.audio
        .play_footstep(run_right(), Some(material))
        .unwrap();
    for _ in 0..4 {
        render_buffer(&mut rig.audio);
    }
    rig.peak.take()
}

mod playback {
    use super::*;

    #[test]
    fn gameplay_phases_play_and_every_other_phase_drops_the_step() {
        let cases = [
            (Phase::MainMenu, false),
            (Phase::SpawnSelect, false),
            (Phase::Loading, false),
            (Phase::Revealing, true),
            (Phase::InGame, true),
        ];
        for (phase, plays) in cases {
            let mut rig = rig();
            enter(&mut rig, phase);
            let peak = audible_after_playing(&mut rig, STONE);
            assert_eq!(peak > 0.0, plays, "{phase:?} peaked at {peak}");
        }
    }

    #[test]
    fn leaving_the_game_stops_new_steps_again() {
        let mut rig = rig();
        enter(&mut rig, Phase::InGame);
        assert!(audible_after_playing(&mut rig, STONE) > 0.0);
        render(&mut rig.audio, SAMPLE_RATE, Duration::from_secs(1));
        enter(&mut rig, Phase::MainMenu);
        rig.peak.take();
        assert!(audible_after_playing(&mut rig, STONE) <= 0.0);
    }

    #[test]
    fn a_system_without_footsteps_drops_the_step() {
        let root = install();
        let (manager_settings, peak) = mock_manager_settings(SAMPLE_RATE);
        let settings = MenuMusicSettings {
            enabled: false,
            file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
            fade: Duration::from_millis(100),
            attenuation_millibels: 300,
        };
        let mut audio = AudioSystem::new(manager_settings, root.path(), settings).unwrap();
        audio.sync(AudioFrame::from(Phase::InGame)).unwrap();
        audio.play_footstep(run_right(), Some(STONE)).unwrap();
        render_buffer(&mut audio);
        assert!(peak.take() <= 0.0);
    }

    #[test]
    fn a_silent_step_plays_nothing() {
        let mut rig = rig();
        enter(&mut rig, Phase::InGame);
        assert!(audible_after_playing(&mut rig, ORPHAN) <= 0.0);
    }

    #[test]
    fn the_mix_gain_scales_the_output() {
        let peak_with = |extra_centidb: i32| {
            let mut quieter = catalog();
            modify_descriptor(&mut quieter, "stone_sound", |d| {
                d.static_attenuation_centidb += extra_centidb;
            });
            let mut rig = rig_with(install(), quieter);
            enter(&mut rig, Phase::InGame);
            audible_after_playing(&mut rig, STONE)
        };
        let loud = peak_with(0);
        let half = peak_with(602);
        assert!(loud > 0.0);
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
    }

    #[test]
    fn the_playback_rate_shortens_the_sound() {
        let audible_late = |shift_pct: i32| {
            let mut shifted = catalog();
            modify_descriptor(&mut shifted, "stone_sound", |d| {
                d.freq_shift_pct = shift_pct;
            });
            let mut rig = rig_with(install(), shifted);
            enter(&mut rig, Phase::InGame);
            rig.audio.play_footstep(run_right(), Some(STONE)).unwrap();
            render(&mut rig.audio, SAMPLE_RATE, Duration::from_millis(300));
            rig.peak.take();
            render(&mut rig.audio, SAMPLE_RATE, Duration::from_millis(100));
            rig.peak.take() > 0.0
        };
        assert!(audible_late(0), "a half second sound still plays at 0.4 s");
        assert!(!audible_late(100), "at double speed it ended at 0.25 s");
    }

    #[test]
    fn a_file_is_decoded_once_and_every_layer_plays_on_the_effect_track() {
        let mut rig = rig();
        enter(&mut rig, Phase::InGame);
        for _ in 0..3 {
            audible_after_playing(&mut rig, STONE);
        }
        assert_eq!(rig.audio.decoded_sfx_count(), 1);
        rig.audio.play_footstep(run_right(), Some(DIRT)).unwrap();
        assert_eq!(rig.audio.decoded_sfx_count(), 3);
    }

    #[test]
    fn a_file_that_vanished_after_startup_is_fatal() {
        let mut rig = rig();
        enter(&mut rig, Phase::InGame);
        std::fs::remove_file(rig.root.path().join("audio").join(STONE_FILE)).unwrap();
        let error = rig
            .audio
            .play_footstep(run_right(), Some(STONE))
            .unwrap_err();
        assert!(matches!(error, AudioError::SfxFile { .. }), "{error}");
    }

    #[test]
    fn a_file_that_cannot_be_decoded_is_fatal() {
        let root = install();
        write_file(
            root.path(),
            STONE_FILE,
            &undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );
        let mut rig = rig_with(root, catalog());
        enter(&mut rig, Phase::InGame);
        let error = rig
            .audio
            .play_footstep(run_right(), Some(STONE))
            .unwrap_err();
        assert!(matches!(error, AudioError::SfxFile { .. }), "{error}");
    }

    #[test]
    fn unusable_footstep_data_is_fatal_when_the_step_plays() {
        let mut broken = catalog();
        modify_descriptor(&mut broken, "stone_sound", |d| d.looping_flags = 0x0800);
        let mut rig = rig_with(install(), broken);
        enter(&mut rig, Phase::InGame);
        let error = rig
            .audio
            .play_footstep(run_right(), Some(STONE))
            .unwrap_err();
        assert!(matches!(error, AudioError::Footsteps(_)), "{error}");
    }
}
