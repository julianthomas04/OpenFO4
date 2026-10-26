use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use super::thunder::FAINTEST_THUNDER_DB;
use super::*;
use crate::assets::db::music::{Comparand, CompareOp};
use crate::assets::db::sound_catalog::{COMPOUND_DESCRIPTOR, SoundDescriptor};
use crate::audio::conditions::{ConditionError, IS_IN_INTERIOR};
use crate::audio::sound_resolve::{LayersError, SoundError};
use crate::audio::test_support::condition::{condition, day};
use crate::audio::test_support::weather::{
    CLEAR, GUSTS, GUSTS_FILE, ONLY_THUNDER, RAIN, RAIN_BED, RAIN_FILES, STORM, STORM_BED,
    STORM_FILE, THUNDER, THUNDER_HIGH_FILES, THUNDER_LOW, THUNDER_LOW_FILE, WIND_BED, WIND_FILE,
    all_files, sound_catalog, steady_gain_db, table,
};
use crate::audio::test_support::{
    HALF_SECOND_SAMPLES, MockAudio, PeakReader, SAMPLE_RATE, SEED, SETTLED_BUFFERS, SharedBuffer,
    mock_manager_settings, render, render_buffer, sine_flac, undecodable_flac, write_audio_file,
};
use crate::audio::{AudioError, AudioFrame, AudioSystem, MenuMusicSettings};
use crate::game_flow::Phase;
use crate::weather::{FadeRange, SoundKind, TransitionSide, WeatherId, WeatherSound};

const GAIN_TOLERANCE: f32 = 1e-3;
const GAME_HOUR: f32 = 12.0;
/// The wind speed of every weather of the tests.
const WIND_SPEED: f32 = 0.5;
const NEVER_FADES: FadeRange = FadeRange {
    begin_fade_in: 0.0,
    end_fade_out: 1.0,
};
const SECOND: Duration = Duration::from_secs(1);
const UNKNOWN_WEATHER: WeatherId = WeatherId::new(9);

fn side(weather: WeatherId, transition: TransitionSide) -> WeatherSide {
    WeatherSide::new(weather, transition, NEVER_FADES, WIND_SPEED)
}

fn settled(weather: WeatherId) -> WeatherAudioInput {
    WeatherAudioInput {
        sides: vec![side(weather, TransitionSide::Settled)],
        strikes: Vec::new(),
        outdoor: true,
    }
}

fn transition(to: WeatherId, from: WeatherId, progress: f32) -> WeatherAudioInput {
    WeatherAudioInput {
        sides: vec![
            side(to, TransitionSide::Incoming { progress }),
            side(from, TransitionSide::Outgoing { progress }),
        ],
        strikes: Vec::new(),
        outdoor: true,
    }
}

fn indoors(input: WeatherAudioInput) -> WeatherAudioInput {
    WeatherAudioInput {
        outdoor: false,
        ..input
    }
}

/// `input` with one more strike.
fn striking(
    mut input: WeatherAudioInput,
    weather: WeatherId,
    delay_s: f32,
    intensity: f32,
) -> WeatherAudioInput {
    input.strikes.push(ThunderStrike {
        weather,
        delay_s,
        intensity,
    });
    input
}

fn sine() -> Vec<u8> {
    sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES)
}

fn install() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for file in all_files() {
        write_audio_file(root.path(), file, &sine());
    }
    root
}

fn remove_file(root: &Path, file_key: &str) {
    std::fs::remove_file(root.join("audio").join(file_key)).unwrap();
}

fn catalog_with(key: &str, change: impl FnOnce(&mut SoundDescriptor)) -> SoundCatalog {
    let mut catalog = sound_catalog();
    change(catalog.descriptors.get_mut(key).unwrap());
    catalog
}

fn weather_audio_with(
    root: &Path,
    catalog: SoundCatalog,
    seed: u64,
) -> Result<WeatherAudio, WeatherAudioError> {
    WeatherAudio::new(catalog, table(), &root.join("audio"), seed)
}

fn weather_audio(root: &Path) -> WeatherAudio {
    weather_audio_with(root, sound_catalog(), SEED).unwrap()
}

fn startup_error(root: &Path, catalog: SoundCatalog) -> WeatherAudioError {
    weather_audio_with(root, catalog, SEED)
        .err()
        .expect("the weather audio should not open")
}

fn update(
    audio: &mut WeatherAudio,
    input: &WeatherAudioInput,
    seconds: f32,
) -> Result<WeatherPlay, WeatherAudioError> {
    audio.update(
        Some(input),
        GameTime::at_hour(GAME_HOUR),
        Duration::from_secs_f32(seconds),
    )
}

fn now_playing(audio: &mut WeatherAudio, input: &WeatherAudioInput) -> WeatherPlay {
    update(audio, input, 0.0).unwrap()
}

fn loop_files(play: &WeatherPlay) -> BTreeSet<&str> {
    play.loops
        .iter()
        .map(|target| target.file_key.as_str())
        .collect()
}

fn thunder_files(play: &WeatherPlay) -> Vec<&str> {
    play.thunder
        .iter()
        .map(|layer| layer.file_key.as_str())
        .collect()
}

fn loop_gain(play: &WeatherPlay, file_key: &str) -> f32 {
    play.loops
        .iter()
        .find(|target| target.file_key == file_key)
        .unwrap_or_else(|| panic!("{file_key} is not asked for in {:?}", play.loops))
        .gain_db
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < GAIN_TOLERANCE,
        "{actual} dB is not {expected} dB"
    );
}

mod loops {
    use super::*;

    #[test]
    fn a_settled_weather_asks_for_its_bed_at_the_steady_gain_of_the_sound() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let play = now_playing(&mut audio, &settled(CLEAR));

        assert_eq!(loop_files(&play), BTreeSet::from([WIND_FILE]));
        assert_close(loop_gain(&play, WIND_FILE), steady_gain_db());
        assert_eq!(play.thunder, []);
    }

    #[test]
    fn every_loop_of_a_weather_follows_its_own_share() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let play = now_playing(&mut audio, &settled(RAIN));

        let rain = loop_files(&play)
            .into_iter()
            .find(|file| RAIN_FILES.contains(file))
            .unwrap();
        assert_eq!(
            loop_files(&play),
            BTreeSet::from([WIND_FILE, rain, GUSTS_FILE])
        );
        assert_close(loop_gain(&play, WIND_FILE), steady_gain_db());
        assert_close(loop_gain(&play, rain), steady_gain_db());
        assert_close(
            loop_gain(&play, GUSTS_FILE),
            steady_gain_db() + 20.0 * WIND_SPEED.log10(),
        );
    }

    #[test]
    fn a_loop_keeps_the_file_it_chose_for_as_long_as_the_game_runs() {
        let root = install();
        let mut audio = weather_audio(root.path());
        let first = now_playing(&mut audio, &settled(RAIN));

        for _ in 0..50 {
            let again = now_playing(&mut audio, &settled(RAIN));
            assert_eq!(loop_files(&again), loop_files(&first));
        }
    }

    #[test]
    fn two_weathers_with_beds_of_their_own_cross_fade_with_the_transition() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let play = now_playing(&mut audio, &transition(STORM, CLEAR, 0.25));

        assert_eq!(loop_files(&play), BTreeSet::from([STORM_FILE, WIND_FILE]));
        assert_close(
            loop_gain(&play, STORM_FILE),
            steady_gain_db() + 20.0 * 0.25_f32.log10(),
        );
        assert_close(
            loop_gain(&play, WIND_FILE),
            steady_gain_db() + 20.0 * 0.75_f32.log10(),
        );
    }

    #[test]
    fn a_bed_both_weathers_share_keeps_its_gain_through_the_whole_transition() {
        let root = install();
        let mut audio = weather_audio(root.path());

        for step in 0..=20 {
            let progress = step as f32 / 20.0;

            let play = now_playing(&mut audio, &transition(RAIN, CLEAR, progress));

            assert_close(loop_gain(&play, WIND_FILE), steady_gain_db());
        }
    }

    #[test]
    fn indoors_no_loop_is_asked_for() {
        let root = install();
        let mut audio = weather_audio(root.path());

        assert_eq!(now_playing(&mut audio, &indoors(settled(RAIN))).loops, []);
    }

    #[test]
    fn a_loop_whose_conditions_fail_is_not_asked_for() {
        let root = install();
        let catalog = catalog_with(WIND_BED, |descriptor| {
            descriptor.conditions = vec![condition(
                IS_IN_INTERIOR,
                CompareOp::Equal,
                Comparand::Value(1.0),
            )];
        });
        let mut audio = weather_audio_with(root.path(), catalog, SEED).unwrap();

        assert_eq!(now_playing(&mut audio, &settled(CLEAR)).loops, []);
    }

    #[test]
    fn the_game_time_reaches_the_conditions_of_a_loop() {
        let root = install();
        let catalog = catalog_with(WIND_BED, |descriptor| descriptor.conditions = day());
        let mut audio = weather_audio_with(root.path(), catalog, SEED).unwrap();
        let input = settled(CLEAR);

        let by_day = audio
            .update(Some(&input), GameTime::at_hour(12.0), Duration::ZERO)
            .unwrap();
        let by_night = audio
            .update(Some(&input), GameTime::at_hour(23.0), Duration::ZERO)
            .unwrap();

        assert_eq!(loop_files(&by_day), BTreeSet::from([WIND_FILE]));
        assert_eq!(by_night.loops, []);
    }

    #[test]
    fn a_loop_without_a_file_left_is_not_asked_for_and_the_others_are() {
        let root = install();
        remove_file(root.path(), WIND_FILE);
        let mut audio = weather_audio(root.path());

        let play = now_playing(&mut audio, &settled(RAIN));

        assert!(!loop_files(&play).contains(WIND_FILE));
        assert!(loop_files(&play).contains(GUSTS_FILE));
    }

    #[test]
    fn a_weather_the_table_does_not_know_is_an_error() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let error = update(&mut audio, &settled(UNKNOWN_WEATHER), 0.0).unwrap_err();

        assert!(
            matches!(error, WeatherAudioError::UnknownWeather(9)),
            "{error:?}"
        );
    }
}

mod strikes {
    use super::*;

    fn rain_strike(delay_s: f32, intensity: f32) -> WeatherAudioInput {
        striking(settled(RAIN), RAIN, delay_s, intensity)
    }

    #[test]
    fn the_thunder_of_a_strike_sounds_after_its_delay_with_every_layer_of_the_sound() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let at_the_flash = update(&mut audio, &rain_strike(2.0, 1.0), 10.0).unwrap();
        let on_its_way = update(&mut audio, &settled(RAIN), 11.9).unwrap();
        let due = update(&mut audio, &settled(RAIN), 12.0).unwrap();
        let afterwards = update(&mut audio, &settled(RAIN), 13.0).unwrap();

        assert_eq!(at_the_flash.thunder, []);
        assert_eq!(on_its_way.thunder, []);
        let files = thunder_files(&due);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(THUNDER_HIGH_FILES.contains(&files[0]), "{files:?}");
        assert_eq!(files[1], THUNDER_LOW_FILE);
        for layer in &due.thunder {
            assert_close(layer.mix.gain_db, steady_gain_db());
        }
        assert_eq!(afterwards.thunder, []);
    }

    #[test]
    fn a_fainter_strike_sounds_quieter_by_its_share_of_the_faintest_gain() {
        let root = install();
        let mut audio = weather_audio(root.path());

        let due = update(&mut audio, &rain_strike(0.0, 0.25), 0.0).unwrap();

        assert_eq!(due.thunder.len(), 2);
        for layer in &due.thunder {
            assert_close(
                layer.mix.gain_db,
                steady_gain_db() + FAINTEST_THUNDER_DB * 0.75,
            );
        }
    }

    #[test]
    fn thunder_that_comes_due_indoors_is_dropped_for_good() {
        let root = install();
        let mut audio = weather_audio(root.path());
        update(&mut audio, &rain_strike(1.0, 1.0), 0.0).unwrap();

        let indoors_when_due = update(&mut audio, &indoors(settled(RAIN)), 1.5).unwrap();
        let outdoors_again = update(&mut audio, &settled(RAIN), 2.0).unwrap();

        assert_eq!(indoors_when_due.thunder, []);
        assert_eq!(outdoors_again.thunder, []);
    }

    #[test]
    fn a_strike_that_hit_while_the_player_was_indoors_sounds_if_they_are_out_in_time() {
        let root = install();
        let mut audio = weather_audio(root.path());
        update(&mut audio, &indoors(rain_strike(2.0, 1.0)), 0.0).unwrap();

        let due = update(&mut audio, &settled(RAIN), 2.0).unwrap();

        assert_eq!(due.thunder.len(), 2);
    }

    #[test]
    fn strikes_sound_in_the_order_they_come_due() {
        let root = install();
        let mut audio = weather_audio(root.path());
        let input = striking(rain_strike(3.0, 0.5), STORM, 1.0, 1.0);
        update(&mut audio, &input, 0.0).unwrap();

        let due = update(&mut audio, &settled(RAIN), 5.0).unwrap();

        let gains: Vec<f32> = due.thunder.iter().map(|layer| layer.mix.gain_db).collect();
        assert_eq!(gains.len(), 4, "{gains:?}");
        assert_close(gains[0], steady_gain_db());
        assert_close(gains[1], steady_gain_db());
        assert_close(gains[2], steady_gain_db() + FAINTEST_THUNDER_DB * 0.5);
        assert_close(gains[3], steady_gain_db() + FAINTEST_THUNDER_DB * 0.5);
    }

    #[test]
    fn a_strike_of_a_weather_without_thunder_sounds_is_silent_and_no_error() {
        let root = install();
        let mut audio = weather_audio(root.path());
        let input = striking(settled(CLEAR), CLEAR, 0.0, 1.0);

        assert_eq!(update(&mut audio, &input, 0.0).unwrap().thunder, []);
    }

    #[test]
    fn a_strike_of_a_weather_the_table_does_not_know_is_an_error_when_it_comes_due() {
        let root = install();
        let mut audio = weather_audio(root.path());
        let input = striking(settled(CLEAR), UNKNOWN_WEATHER, 1.0, 1.0);

        assert_eq!(update(&mut audio, &input, 0.0).unwrap().thunder, []);
        let error = update(&mut audio, &settled(CLEAR), 1.0).unwrap_err();

        assert!(
            matches!(error, WeatherAudioError::UnknownWeather(9)),
            "{error:?}"
        );
    }

    #[test]
    fn without_a_weather_the_thunder_on_its_way_is_dropped_and_no_loop_is_asked_for() {
        let root = install();
        let mut audio = weather_audio(root.path());
        update(&mut audio, &rain_strike(1.0, 1.0), 0.0).unwrap();

        let none = audio
            .update(
                None,
                GameTime::at_hour(GAME_HOUR),
                Duration::from_millis(500),
            )
            .unwrap();
        let later = update(&mut audio, &settled(RAIN), 3.0).unwrap();

        assert_eq!(none, WeatherPlay::default());
        assert_eq!(later.thunder, []);
    }

    #[test]
    fn the_layers_of_a_thunder_sound_each_pick_a_file_of_their_own() {
        let root = install();
        let mut high_files = BTreeSet::new();
        for seed in 0..30 {
            let mut audio = weather_audio_with(root.path(), sound_catalog(), seed).unwrap();
            let due = update(&mut audio, &rain_strike(0.0, 1.0), 0.0).unwrap();
            high_files.insert(thunder_files(&due)[0].to_string());
        }

        assert_eq!(
            high_files,
            THUNDER_HIGH_FILES.iter().map(ToString::to_string).collect()
        );
    }

    #[test]
    fn a_thunder_sound_with_a_missing_layer_file_still_plays_the_other_layer() {
        let root = install();
        remove_file(root.path(), THUNDER_LOW_FILE);
        let mut audio = weather_audio(root.path());

        let due = update(&mut audio, &rain_strike(0.0, 1.0), 0.0).unwrap();

        assert_eq!(due.thunder.len(), 1);
        assert!(THUNDER_HIGH_FILES.contains(&thunder_files(&due)[0]));
    }
}

mod startup {
    use super::*;

    #[test]
    fn a_complete_install_opens_and_misses_no_file() {
        let root = install();

        let audio = weather_audio(root.path());

        assert_eq!(audio.missing_files(), &HashSet::new());
    }

    #[test]
    fn a_missing_audio_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();

        let error = startup_error(root.path(), sound_catalog());

        assert!(
            matches!(error, WeatherAudioError::AudioDirectoryMissing(_)),
            "{error:?}"
        );
    }

    #[test]
    fn a_missing_file_is_reported_once_and_never_chosen() {
        let root = install();
        remove_file(root.path(), RAIN_FILES[0]);
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");

        let audio = weather_audio(root.path());

        let logged = log.drain();
        let missing_lines: Vec<&str> = logged
            .lines()
            .filter(|line| line.contains("is missing"))
            .collect();
        assert_eq!(missing_lines.len(), 1, "{logged}");
        assert!(missing_lines[0].contains(RAIN_FILES[0]), "{logged}");
        assert_eq!(
            audio.missing_files(),
            &HashSet::from([RAIN_FILES[0].to_string()])
        );
        for seed in 0..10 {
            let mut audio = weather_audio_with(root.path(), sound_catalog(), seed).unwrap();
            let play = now_playing(&mut audio, &settled(RAIN));
            assert!(loop_files(&play).contains(RAIN_FILES[1]), "{seed}");
            assert!(!loop_files(&play).contains(RAIN_FILES[0]), "{seed}");
        }
    }

    #[test]
    fn a_sound_left_without_files_is_logged_once_at_startup_and_never_again() {
        let root = install();
        remove_file(root.path(), WIND_FILE);
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");

        let mut audio = weather_audio(root.path());
        for _ in 0..3 {
            now_playing(&mut audio, &settled(CLEAR));
        }

        let logged = log.drain();
        assert_eq!(logged.lines().count(), 2, "{logged}");
        assert!(logged.contains("wind_bed has no file left"), "{logged}");
    }

    #[test]
    fn a_file_whose_header_cannot_be_opened_is_an_error() {
        let root = install();
        write_audio_file(root.path(), GUSTS_FILE, &[]);

        let error = startup_error(root.path(), sound_catalog());

        assert!(
            matches!(error, WeatherAudioError::UnreadableFile { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_descriptor_the_catalog_does_not_have_is_an_error_naming_it() {
        let root = install();
        let mut catalog = sound_catalog();
        catalog.descriptors.remove(STORM_BED);

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Sound(SoundError::UnknownRecord { key, .. }) if key == STORM_BED),
            "{error:?}"
        );
    }

    #[test]
    fn only_the_descriptors_of_the_table_have_to_be_in_the_catalog() {
        let root = install();
        let mut catalog = sound_catalog();
        catalog.descriptors.remove(STORM_BED);
        let table = WeatherSoundTable::new(vec![vec![WeatherSound {
            kind: SoundKind::Default,
            descriptor: WIND_BED.to_string(),
        }]]);

        let audio = WeatherAudio::new(catalog, table, &root.path().join("audio"), SEED);

        assert!(audio.is_ok());
    }

    #[test]
    fn a_loop_that_is_a_compound_sound_is_an_error() {
        let root = install();
        let catalog = catalog_with(WIND_BED, |descriptor| {
            descriptor.descriptor_type = COMPOUND_DESCRIPTOR;
        });

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Sound(SoundError::UnsupportedDescriptor { descriptor, .. }) if descriptor == WIND_BED),
            "{error:?}"
        );
    }

    #[test]
    fn a_thunder_layer_that_loops_is_an_error() {
        let root = install();
        let catalog = catalog_with(THUNDER_LOW, |descriptor| descriptor.looping_flags = 0x0800);

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Layers(LayersError::Sound(SoundError::Looping(descriptor))) if descriptor == THUNDER_LOW),
            "{error:?}"
        );
    }

    #[test]
    fn a_thunder_sound_that_contains_itself_is_an_error() {
        let root = install();
        let catalog = catalog_with(THUNDER, |descriptor| {
            descriptor.children.push(THUNDER.to_string());
        });

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Layers(LayersError::Cycle(descriptor)) if descriptor == THUNDER),
            "{error:?}"
        );
    }

    #[test]
    fn a_loop_condition_that_cannot_be_evaluated_is_an_error() {
        let root = install();
        let catalog = catalog_with(RAIN_BED, |descriptor| {
            descriptor.conditions = vec![condition(9999, CompareOp::Equal, Comparand::Value(0.0))];
        });

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(
                error,
                WeatherAudioError::Condition(ConditionError::UnsupportedFunction(9999))
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_loop_that_is_silent_at_startup_is_still_checked() {
        let root = install();
        let catalog = catalog_with(WIND_BED, |descriptor| {
            descriptor.conditions = vec![condition(
                IS_IN_INTERIOR,
                CompareOp::Equal,
                Comparand::Value(1.0),
            )];
            descriptor.category = Some("no such category".to_string());
        });

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Sound(SoundError::Mix { descriptor, .. }) if descriptor == WIND_BED),
            "{error:?}"
        );
    }

    #[test]
    fn a_sound_whose_mix_cannot_be_decided_is_an_error() {
        let root = install();
        let catalog = catalog_with(GUSTS, |descriptor| {
            descriptor.category = Some("no such category".to_string());
        });

        let error = startup_error(root.path(), catalog);

        assert!(
            matches!(&error, WeatherAudioError::Sound(SoundError::Mix { descriptor, .. }) if descriptor == GUSTS),
            "{error:?}"
        );
    }
}

const FADE: Duration = Duration::from_millis(100);
/// Enough rendering for a fade to be over and the loops to sit at their gain.
const SETTLING: Duration = Duration::from_millis(400);

struct Rig {
    _root: tempfile::TempDir,
    audio: MockAudio,
    peak: PeakReader,
}

fn menu_settings() -> MenuMusicSettings {
    MenuMusicSettings {
        enabled: false,
        file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
        fade: FADE,
        attenuation_millibels: 300,
    }
}

fn frame(phase: Phase, dt: Duration, weather: Option<WeatherAudioInput>) -> AudioFrame {
    AudioFrame {
        phase,
        dt,
        time: GameTime::at_hour(GAME_HOUR),
        place: None,
        weather,
    }
}

impl Rig {
    fn new() -> Self {
        Self::with(install())
    }

    fn with(root: tempfile::TempDir) -> Self {
        let (manager_settings, peak) = mock_manager_settings(SAMPLE_RATE);
        let mut audio = AudioSystem::new(manager_settings, root.path(), menu_settings()).unwrap();
        audio.set_weather(weather_audio(root.path()));
        Self {
            _root: root,
            audio,
            peak,
        }
    }

    fn try_sync(
        &mut self,
        phase: Phase,
        dt: Duration,
        weather: WeatherAudioInput,
    ) -> Result<(), AudioError> {
        self.audio.sync(frame(phase, dt, Some(weather)))
    }

    fn sync(&mut self, phase: Phase, weather: WeatherAudioInput) {
        self.try_sync(phase, Duration::ZERO, weather).unwrap();
    }

    fn in_game(&mut self, weather: WeatherAudioInput) {
        self.sync(Phase::InGame, weather);
    }

    /// The loudest sample after the fades of the commands so far are over.
    fn peak_after_settling(&mut self) -> f32 {
        render(&mut self.audio, SAMPLE_RATE, SETTLING);
        self.peak.take();
        for _ in 0..SETTLED_BUFFERS {
            render_buffer(&mut self.audio);
        }
        self.peak.take()
    }

    /// The loudest sample of the next few buffers.
    fn peak_now(&mut self) -> f32 {
        self.peak.take();
        for _ in 0..4 {
            render_buffer(&mut self.audio);
        }
        self.peak.take()
    }
}

mod loop_playback {
    use super::*;

    #[test]
    fn the_weather_plays_in_the_gameplay_phases_only() {
        let cases = [
            (Phase::MainMenu, false),
            (Phase::SpawnSelect, false),
            (Phase::Loading, false),
            (Phase::Revealing, true),
            (Phase::InGame, true),
        ];
        for (phase, plays) in cases {
            let mut rig = Rig::new();
            rig.sync(phase, settled(CLEAR));

            let peak = rig.peak_after_settling();

            assert_eq!(peak > 0.0, plays, "{phase:?} peaked at {peak}");
            assert_eq!(
                rig.audio.playing_loop_files(),
                if plays { vec![WIND_FILE] } else { vec![] },
                "{phase:?}"
            );
        }
    }

    #[test]
    fn leaving_the_world_fades_the_weather_out() {
        let mut rig = Rig::new();
        rig.in_game(settled(CLEAR));
        assert!(rig.peak_after_settling() > 0.0);

        rig.sync(Phase::MainMenu, settled(CLEAR));

        assert_eq!(rig.peak_after_settling(), 0.0);
        assert_eq!(rig.audio.playing_loop_files(), Vec::<&str>::new());
    }

    #[test]
    fn a_frame_without_weather_fades_the_weather_out() {
        let mut rig = Rig::new();
        rig.in_game(settled(CLEAR));
        assert!(rig.peak_after_settling() > 0.0);

        rig.audio
            .sync(frame(Phase::InGame, Duration::ZERO, None))
            .unwrap();

        assert_eq!(rig.peak_after_settling(), 0.0);
    }

    #[test]
    fn the_weather_fades_out_when_the_player_goes_indoors_and_back_in_when_they_come_out() {
        let mut rig = Rig::new();
        rig.in_game(settled(CLEAR));
        let outdoors = rig.peak_after_settling();
        assert!(outdoors > 0.0);

        rig.in_game(indoors(settled(CLEAR)));
        let inside = rig.peak_after_settling();
        rig.in_game(settled(CLEAR));
        let out_again = rig.peak_after_settling();

        assert_eq!(inside, 0.0);
        assert!(
            (out_again / outdoors - 1.0).abs() < 0.02,
            "{out_again} against {outdoors}"
        );
    }

    #[test]
    fn a_transition_plays_both_beds_and_ends_with_the_new_one_alone() {
        let mut rig = Rig::new();
        rig.in_game(transition(STORM, CLEAR, 0.5));
        rig.peak_after_settling();
        assert_eq!(rig.audio.playing_loop_files(), [STORM_FILE, WIND_FILE]);

        rig.in_game(settled(STORM));

        assert!(rig.peak_after_settling() > 0.0);
        assert_eq!(rig.audio.playing_loop_files(), [STORM_FILE]);
    }

    #[test]
    fn a_weather_the_table_does_not_know_is_an_error_of_the_weather_sounds() {
        let mut rig = Rig::new();

        let error = rig
            .try_sync(Phase::InGame, Duration::ZERO, settled(UNKNOWN_WEATHER))
            .unwrap_err();

        assert!(
            matches!(
                error,
                AudioError::Weather(WeatherAudioError::UnknownWeather(9))
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_system_without_weather_sounds_ignores_the_weather_of_the_frames() {
        let root = install();
        let (manager_settings, _peak) = mock_manager_settings(SAMPLE_RATE);
        let mut audio = AudioSystem::new(manager_settings, root.path(), menu_settings()).unwrap();

        audio
            .sync(frame(Phase::InGame, Duration::ZERO, Some(settled(CLEAR))))
            .unwrap();

        assert_eq!(audio.playing_loop_files(), Vec::<&str>::new());
    }

    #[test]
    fn a_damaged_loop_file_is_an_error_when_it_first_plays() {
        let root = install();
        write_audio_file(
            root.path(),
            WIND_FILE,
            &undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );
        let mut rig = Rig::with(root);

        let error = rig
            .try_sync(Phase::InGame, Duration::ZERO, settled(CLEAR))
            .unwrap_err();

        assert!(matches!(error, AudioError::SfxFile { .. }), "{error:?}");
    }
}

mod thunder_playback {
    use super::*;

    fn only_thunder_in(delay: Duration) -> WeatherAudioInput {
        striking(
            settled(ONLY_THUNDER),
            ONLY_THUNDER,
            delay.as_secs_f32(),
            1.0,
        )
    }

    fn pass(rig: &mut Rig, time: Duration, weather: WeatherAudioInput) {
        rig.try_sync(Phase::InGame, time, weather).unwrap();
    }

    #[test]
    fn the_thunder_is_heard_once_its_delay_has_passed() {
        let mut rig = Rig::new();

        pass(&mut rig, Duration::ZERO, only_thunder_in(SECOND));
        let at_the_flash = rig.peak_now();
        pass(&mut rig, Duration::from_millis(900), settled(ONLY_THUNDER));
        let before_the_thunder = rig.peak_now();
        pass(&mut rig, Duration::from_millis(100), settled(ONLY_THUNDER));
        let at_the_thunder = rig.peak_now();

        assert_eq!(at_the_flash, 0.0);
        assert_eq!(before_the_thunder, 0.0);
        assert!(at_the_thunder > 0.0);
    }

    #[test]
    fn the_thunder_is_not_heard_when_the_player_is_indoors_as_it_comes_due() {
        let mut rig = Rig::new();

        pass(&mut rig, Duration::ZERO, only_thunder_in(SECOND));
        pass(&mut rig, SECOND, indoors(settled(ONLY_THUNDER)));

        assert_eq!(rig.peak_now(), 0.0);
    }

    #[test]
    fn the_thunder_on_its_way_is_dropped_when_the_world_is_left() {
        let mut rig = Rig::new();
        pass(&mut rig, Duration::ZERO, only_thunder_in(SECOND));

        rig.sync(Phase::MainMenu, settled(ONLY_THUNDER));
        pass(&mut rig, SECOND, settled(ONLY_THUNDER));

        assert_eq!(rig.peak_now(), 0.0);
    }
}
