use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::PlaybackState;

use super::test_support::{
    HALF_SECOND_SAMPLES, MockAudio, PeakReader, SAMPLE_RATE, SETTLED_BUFFERS,
    mock_manager_settings, render, render_buffer, sine_flac, undecodable_flac, wait_until,
};
use super::{AudioError, AudioSystem, MenuMusicSettings};
use crate::game_flow::Phase;

const FADE: Duration = Duration::from_millis(100);
const LONG_FADE: Duration = Duration::from_secs(5);
const ATTENUATION_MILLIBELS: u32 = 600;
const MUSIC_FILE: &str = "audio/music/special/mus_maintheme.flac";

fn install_with(music: &[u8]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write_music(root.path(), music);
    root
}

fn install_with_music() -> tempfile::TempDir {
    install_with(&sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES))
}

fn music_file(root: &Path) -> PathBuf {
    root.join(MUSIC_FILE)
}

fn write_music(root: &Path, music: &[u8]) {
    let file = music_file(root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, music).unwrap();
}

fn settings(enabled: bool, attenuation_millibels: u32) -> MenuMusicSettings {
    MenuMusicSettings {
        enabled,
        file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
        fade: FADE,
        attenuation_millibels,
    }
}

fn open_metered(
    root: &Path,
    settings: MenuMusicSettings,
) -> Result<(MockAudio, PeakReader), AudioError> {
    let (manager_settings, reader) = mock_manager_settings(SAMPLE_RATE);
    AudioSystem::new(manager_settings, root, settings).map(|audio| (audio, reader))
}

fn open(root: &Path, enabled: bool) -> Result<MockAudio, AudioError> {
    open_metered(root, settings(enabled, 300)).map(|(audio, _)| audio)
}

/// The loudest sample of the first buffer in which the music is audible, and of a stretch of
/// buffers after the fade-in, for the menu music played with the given fade and attenuation.
/// The decode thread runs beside the render loop, so the wait for the first sound is bounded
/// polling; a fade much longer than that wait keeps the first buffer near the fade's start.
fn first_and_settled_peaks(fade: Duration, attenuation_millibels: u32) -> (f32, f32) {
    let root = install_with(&sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES));
    let mut settings = settings(true, attenuation_millibels);
    settings.fade = fade;
    let (mut audio, peak) = open_metered(root.path(), settings).unwrap();
    audio.sync(Phase::MainMenu).unwrap();

    let mut first = 0.0;
    wait_until("the music to become audible", || {
        render_buffer(&mut audio);
        first = peak.take();
        first > 0.0
    });
    render(&mut audio, SAMPLE_RATE, fade);
    peak.take();
    let mut settled = 0.0_f32;
    wait_until("the settled music to become audible", || {
        render_buffer(&mut audio);
        settled = settled.max(peak.take());
        settled > 0.0
    });
    for _ in 0..SETTLED_BUFFERS {
        render_buffer(&mut audio);
        settled = settled.max(peak.take());
    }
    (first, settled)
}

#[test]
fn the_music_wraps_back_to_its_start_instead_of_stopping_at_its_end() {
    let root = install_with_music();
    let mut audio = open(root.path(), true).unwrap();
    audio.sync(Phase::MainMenu).unwrap();

    let mut furthest = 0.0_f64;
    wait_until("the music position to wrap around", || {
        render_buffer(&mut audio);
        assert_ne!(
            audio.music_state(),
            Some(PlaybackState::Stopped),
            "the music stopped at its end"
        );
        let position = audio.music_position().unwrap();
        let wrapped = position < furthest;
        furthest = furthest.max(position);
        wrapped
    });

    assert_eq!(audio.music_state(), Some(PlaybackState::Playing));
}

#[test]
fn the_music_track_is_attenuated_by_the_configured_millibels() {
    let (_, unattenuated) = first_and_settled_peaks(Duration::ZERO, 0);
    let (_, attenuated) = first_and_settled_peaks(Duration::ZERO, ATTENUATION_MILLIBELS);

    assert!(unattenuated > 0.0);
    let expected = 10f32.powf(-(ATTENUATION_MILLIBELS as f32) / 2000.0);
    let ratio = attenuated / unattenuated;
    assert!(
        (ratio / expected - 1.0).abs() < 5e-3,
        "ratio {ratio}, expected {expected}"
    );
}

#[test]
fn the_music_fades_in_from_silence() {
    let (early, settled) = first_and_settled_peaks(LONG_FADE, 0);

    assert!(settled > 0.0);
    assert!(
        early < settled / 10.0,
        "early peak {early} is not far below the settled peak {settled}"
    );
}

#[test]
fn spawn_select_keeps_playing_and_loading_fades_out_to_a_stop() {
    let root = install_with_music();
    let mut audio = open(root.path(), true).unwrap();
    audio.sync(Phase::MainMenu).unwrap();
    render(&mut audio, SAMPLE_RATE, FADE);

    audio.sync(Phase::SpawnSelect).unwrap();
    render(&mut audio, SAMPLE_RATE, FADE * 3);
    assert_eq!(audio.music_state(), Some(PlaybackState::Playing));

    audio.sync(Phase::Loading).unwrap();
    render(&mut audio, SAMPLE_RATE, Duration::from_millis(1));
    assert_eq!(audio.music_state(), Some(PlaybackState::Stopping));
    render(&mut audio, SAMPLE_RATE, FADE * 2);
    assert_eq!(audio.music_state(), Some(PlaybackState::Stopped));
}

#[test]
fn returning_to_the_menu_plays_again() {
    let root = install_with_music();
    let mut audio = open(root.path(), true).unwrap();
    audio.sync(Phase::MainMenu).unwrap();
    audio.sync(Phase::Loading).unwrap();
    render(&mut audio, SAMPLE_RATE, FADE * 3);

    audio.sync(Phase::MainMenu).unwrap();
    render(&mut audio, SAMPLE_RATE, Duration::from_millis(1));

    assert_eq!(audio.music_state(), Some(PlaybackState::Playing));
}

#[test]
fn replaying_during_a_fade_out_keeps_the_fading_playback_until_it_stops() {
    let root = install_with_music();
    let mut audio = open(root.path(), true).unwrap();
    audio.sync(Phase::MainMenu).unwrap();
    audio.sync(Phase::Loading).unwrap();

    audio.sync(Phase::MainMenu).unwrap();
    assert_eq!(audio.retiring_music_count(), 1);

    render(&mut audio, SAMPLE_RATE, FADE * 3);
    audio.sync(Phase::MainMenu).unwrap();
    assert_eq!(audio.retiring_music_count(), 0);
}

#[test]
fn a_direct_start_and_disabled_music_never_play() {
    let root = install_with_music();
    let mut direct = open(root.path(), true).unwrap();
    direct.sync(Phase::Loading).unwrap();
    direct.sync(Phase::InGame).unwrap();
    assert_eq!(direct.music_state(), None);

    let mut disabled = open(root.path(), false).unwrap();
    disabled.sync(Phase::MainMenu).unwrap();
    assert_eq!(disabled.music_state(), None);
}

#[test]
fn missing_music_is_an_error_naming_the_file() {
    let root = tempfile::tempdir().unwrap();
    let error = open(root.path(), true).err().unwrap();
    assert!(
        matches!(&error, AudioError::MusicFile { path, .. } if path.ends_with("mus_maintheme.flac")),
        "{error}"
    );
}

#[test]
fn undecodable_music_is_an_error() {
    let root = install_with(b"not audio at all");
    assert!(matches!(
        open(root.path(), true).err().unwrap(),
        AudioError::MusicFile { .. }
    ));
}

#[test]
fn disabled_music_does_not_need_the_file() {
    let root = tempfile::tempdir().unwrap();
    assert!(open(root.path(), false).is_ok());
}

#[test]
fn a_music_file_name_that_leaves_the_audio_folder_is_an_error() {
    let root = install_with_music();
    let mut escaping = settings(true, 300);
    escaping.file = "\\Data\\..\\..\\outside.xwm".to_string();
    assert!(matches!(
        open_metered(root.path(), escaping).err().unwrap(),
        AudioError::MusicPath(_)
    ));
}

#[test]
fn music_that_vanishes_before_the_menu_is_entered_is_a_fatal_error_from_sync() {
    let root = install_with_music();
    let mut audio = open(root.path(), true).unwrap();
    std::fs::remove_file(music_file(root.path())).unwrap();

    assert!(matches!(
        audio.sync(Phase::MainMenu).unwrap_err(),
        AudioError::MusicFile { .. }
    ));
}

#[test]
fn music_that_fails_to_decode_while_playing_is_a_fatal_error_from_sync() {
    let root = install_with(&undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES));
    let mut audio = open(root.path(), true).unwrap();
    audio.sync(Phase::MainMenu).unwrap();

    let mut result = Ok(());
    wait_until("the decode error to surface", || {
        result = audio.sync(Phase::MainMenu);
        result.is_err()
    });

    assert!(matches!(result, Err(AudioError::MusicStream { .. })));
}
