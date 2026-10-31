//! The world music through kira's mock backend: crossfades, plan timing, the handover from the
//! menu music, and the fatal errors. Plan decisions have their own tests in `world_music`.

use std::time::Duration;

use kira::sound::PlaybackState;

use super::conditions::GameTime;
use super::scheduler::{FLAG_ABRUPT_TRANSITION, FLAG_CYCLE_TRACKS, PlayPlan};
use super::test_support::catalog::{
    MASTER_CATEGORY_MULTIPLIER, MUSIC_CATEGORY_MULTIPLIER, exterior_place, palette, silent, single,
    timed_music_type as typed, world_catalog,
};
use super::test_support::condition::{day, night};
use super::test_support::{
    HALF_SECOND_SAMPLES, MockAudio, PeakReader, SAMPLE_RATE, SEED, SETTLED_BUFFERS,
    mock_manager_settings, render, render_buffer, sine_flac, undecodable_flac, wait_until,
    write_audio_file,
};
use super::world_music::WorldMusic;
use super::{AudioError, AudioFrame, AudioSystem, MenuMusicSettings};
use crate::assets::db::music::MusicCatalog;
use crate::game_flow::Phase;

const MENU_FADE: Duration = Duration::from_millis(100);
const CYCLES: u32 = FLAG_CYCLE_TRACKS;
const ABRUPT: u32 = FLAG_ABRUPT_TRANSITION;
const MENU_FILE: &str = "music/special/mus_maintheme.flac";

struct World {
    root: tempfile::TempDir,
    audio: MockAudio,
    peak: PeakReader,
}

/// A system with the menu music and the catalog's world music; every file the catalog's single
/// tracks name is a half second of sine.
fn world_with(catalog: MusicCatalog) -> World {
    let root = tempfile::tempdir().unwrap();
    let sine = sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES);
    write_audio_file(root.path(), MENU_FILE, &sine);
    for track in &catalog.tracks {
        if let crate::assets::db::music::MusicTrackKind::Single { file_key, .. } = &track.kind {
            write_audio_file(root.path(), file_key, &sine);
        }
    }
    let (manager_settings, peak) = mock_manager_settings(SAMPLE_RATE);
    let settings = MenuMusicSettings {
        enabled: true,
        file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
        fade: MENU_FADE,
        attenuation_millibels: 300,
    };
    let mut audio = AudioSystem::new(manager_settings, root.path(), settings).unwrap();
    let music = WorldMusic::new(catalog, &root.path().join("audio"), SEED).unwrap();
    audio.set_world_music(music);
    World { root, audio, peak }
}

fn frame(phase: Phase, seconds: f32, hour: f32, worldspace: Option<&str>) -> AudioFrame {
    AudioFrame {
        phase,
        dt: Duration::from_secs_f32(seconds),
        time: GameTime::at_hour(hour),
        place: worldspace.map(exterior_place),
        weather: None,
    }
}

fn in_world(audio: &mut MockAudio, worldspace: &str) {
    audio
        .sync(frame(Phase::InGame, 0.0, 9.0, Some(worldspace)))
        .unwrap();
}

fn tick(audio: &mut MockAudio, seconds: f32, hour: f32) {
    audio
        .sync(frame(Phase::InGame, seconds, hour, None))
        .unwrap();
}

fn plan(audio: &MockAudio) -> Option<PlayPlan> {
    audio.world_music().unwrap().current_plan().cloned()
}

fn single_plan(file_key: &str) -> PlayPlan {
    PlayPlan::Single {
        file_key: file_key.to_string(),
    }
}

fn two_types(fade_a: f32, fade_b: f32) -> MusicCatalog {
    world_catalog(
        &[("ws1", "A"), ("ws2", "B")],
        vec![
            typed("A", ABRUPT, fade_a, &["music/a.flac"]),
            typed("B", ABRUPT, fade_b, &["music/b.flac"]),
        ],
        vec![
            single("music/a.flac", vec![]),
            single("music/b.flac", vec![]),
        ],
    )
}

/// Types that are not abrupt, each on its own worldspace.
fn queueing_types() -> MusicCatalog {
    world_catalog(
        &[("ws1", "A"), ("ws2", "B"), ("ws3", "C"), ("ws4", "Quiet")],
        vec![
            typed("A", 0, 0.1, &["music/a.flac"]),
            typed("B", 0, 0.1, &["music/b.flac"]),
            typed("C", 0, 0.1, &["music/c.flac"]),
            typed("Quiet", 0, 0.1, &["pause"]),
            typed("P", 0, 0.1, &["pal"]),
        ],
        vec![
            single("music/a.flac", vec![]),
            single("music/b.flac", vec![]),
            single("music/c.flac", vec![]),
            silent("pause", 30.0),
            palette("pal", &[&["music/p.flac"]], 10.0, Some(0.2)),
            single("music/p.flac", vec![]),
        ],
    )
}

/// Ticks until the sound of the previous plan has been handed over to the retiring ones, which
/// is the moment the next plan starts.
fn wait_for_next_plan(world: &mut World) {
    wait_until(
        "the playing sound to end and the next plan to start",
        || {
            render_buffer(&mut world.audio);
            tick(&mut world.audio, 0.0, 9.0);
            world.audio.retiring_music_count() == 1
        },
    );
}

#[test]
fn a_type_change_fades_the_old_sounds_over_the_new_types_fade_and_starts_the_new_plan() {
    let mut world = world_with(two_types(5.0, 0.1));
    in_world(&mut world.audio, "ws1");
    assert_eq!(world.audio.world_sound_count(), 1);
    assert_eq!(plan(&world.audio), Some(single_plan("music/a.flac")));

    in_world(&mut world.audio, "ws2");

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
    assert_eq!(world.audio.world_sound_count(), 1);
    assert_eq!(world.audio.retiring_music_count(), 1);

    render(&mut world.audio, SAMPLE_RATE, Duration::from_millis(300));
    tick(&mut world.audio, 0.0, 9.0);
    assert_eq!(
        world.audio.retiring_music_count(),
        0,
        "the old sound did not stop within the new type's 0.1 s fade"
    );
}

#[test]
fn an_abrupt_type_change_fades_the_playing_sound_even_when_it_has_not_ended() {
    let mut world = world_with(two_types(0.1, 0.1));
    in_world(&mut world.audio, "ws1");

    in_world(&mut world.audio, "ws2");

    assert_eq!(world.audio.retiring_music_count(), 1);
}

#[test]
fn a_type_change_lets_the_playing_track_finish_and_then_starts_the_new_type() {
    let mut world = world_with(queueing_types());
    in_world(&mut world.audio, "ws1");

    in_world(&mut world.audio, "ws2");
    assert_eq!(world.audio.world_sound_count(), 1);
    assert_eq!(
        world.audio.retiring_music_count(),
        0,
        "the playing track was faded out"
    );

    render(&mut world.audio, SAMPLE_RATE, Duration::from_millis(200));
    tick(&mut world.audio, 0.0, 9.0);
    assert_eq!(world.audio.retiring_music_count(), 0, "the track was cut");

    wait_for_next_plan(&mut world);
    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn the_latest_type_change_wins_while_one_waits() {
    let mut world = world_with(queueing_types());
    in_world(&mut world.audio, "ws1");
    in_world(&mut world.audio, "ws2");
    in_world(&mut world.audio, "ws3");

    wait_for_next_plan(&mut world);

    assert_eq!(plan(&world.audio), Some(single_plan("music/c.flac")));
    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn a_type_change_during_a_silent_plan_starts_the_new_type_at_once() {
    let mut world = world_with(queueing_types());
    in_world(&mut world.audio, "ws4");
    assert_eq!(world.audio.world_sound_count(), 0);

    in_world(&mut world.audio, "ws2");

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn a_type_change_waits_for_the_sounds_of_a_palette_not_for_its_whole_duration() {
    let mut catalog = queueing_types();
    catalog.worldspace_music.insert("ws5".into(), "P".into());
    let mut world = world_with(catalog);
    in_world(&mut world.audio, "ws5");

    in_world(&mut world.audio, "ws2");
    assert_eq!(world.audio.retiring_music_count(), 0);

    wait_for_next_plan(&mut world);
    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
}

#[test]
fn a_place_without_music_lets_the_playing_track_finish_and_then_falls_silent() {
    let mut world = world_with(queueing_types());
    in_world(&mut world.audio, "ws1");

    in_world(&mut world.audio, "nowhere");
    assert_eq!(world.audio.world_sound_count(), 1);
    assert_eq!(world.audio.retiring_music_count(), 0);

    wait_for_next_plan(&mut world);
    assert_eq!(plan(&world.audio), None);
    assert_eq!(world.audio.world_sound_count(), 0);
}

fn cycling_day_night() -> MusicCatalog {
    world_catalog(
        &[("ws1", "A")],
        vec![typed(
            "A",
            CYCLES,
            0.1,
            &["music/day.flac", "music/night.flac"],
        )],
        vec![
            single("music/day.flac", day()),
            single("music/night.flac", night()),
        ],
    )
}

#[test]
fn when_a_single_track_ends_a_cycling_type_picks_the_next_track() {
    let mut world = world_with(cycling_day_night());
    in_world(&mut world.audio, "ws1");
    assert_eq!(plan(&world.audio), Some(single_plan("music/day.flac")));

    wait_until("the next track to start after the first ended", || {
        render_buffer(&mut world.audio);
        tick(&mut world.audio, 0.0, 23.0);
        plan(&world.audio) == Some(single_plan("music/night.flac"))
    });

    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn when_a_single_track_ends_a_type_that_does_not_cycle_falls_silent() {
    let mut world = world_with(two_types(0.1, 0.1));
    in_world(&mut world.audio, "ws1");

    wait_until("the track to end and the plan to be dropped", || {
        render_buffer(&mut world.audio);
        tick(&mut world.audio, 0.0, 9.0);
        plan(&world.audio).is_none()
    });

    assert_eq!(world.audio.world_sound_count(), 0);
}

#[test]
fn a_silent_track_lasts_its_seconds_and_then_the_next_plan_starts() {
    let mut pause = silent("pause", 2.0);
    pause.conditions = day();
    let catalog = world_catalog(
        &[("ws1", "A")],
        vec![typed("A", CYCLES, 0.1, &["pause", "music/night.flac"])],
        vec![pause, single("music/night.flac", night())],
    );
    let mut world = world_with(catalog);
    in_world(&mut world.audio, "ws1");
    assert_eq!(plan(&world.audio), Some(PlayPlan::Silent { seconds: 2.0 }));
    assert_eq!(world.audio.world_sound_count(), 0);

    tick(&mut world.audio, 1.9, 23.0);
    assert_eq!(plan(&world.audio), Some(PlayPlan::Silent { seconds: 2.0 }));

    tick(&mut world.audio, 0.2, 23.0);
    assert_eq!(plan(&world.audio), Some(single_plan("music/night.flac")));
    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn a_palette_plays_all_layers_together_and_stops_them_when_its_duration_is_over() {
    let catalog = world_catalog(
        &[("ws1", "A")],
        vec![typed("A", 0, 0.1, &["pal"])],
        vec![
            palette(
                "pal",
                &[&["music/l0.flac"], &["music/l1.flac"]],
                1.0,
                Some(0.2),
            ),
            single("music/l0.flac", vec![]),
            single("music/l1.flac", vec![]),
        ],
    );
    let mut world = world_with(catalog);
    in_world(&mut world.audio, "ws1");
    assert_eq!(world.audio.world_sound_count(), 2);

    tick(&mut world.audio, 0.9, 9.0);
    assert_eq!(
        world.audio.world_sound_count(),
        2,
        "stopped before its duration"
    );

    tick(&mut world.audio, 0.2, 9.0);
    assert_eq!(world.audio.world_sound_count(), 0);
    assert_eq!(world.audio.retiring_music_count(), 2);

    render(&mut world.audio, SAMPLE_RATE, Duration::from_millis(500));
    tick(&mut world.audio, 0.0, 9.0);
    assert_eq!(world.audio.retiring_music_count(), 0);
}

#[test]
fn the_menu_music_stops_for_loading_and_the_world_music_starts_at_revealing() {
    let mut world = world_with(two_types(0.1, 0.1));
    world.audio.sync(Phase::MainMenu).unwrap();
    render(&mut world.audio, SAMPLE_RATE, MENU_FADE * 2);
    assert_eq!(world.audio.music_state(), Some(PlaybackState::Playing));

    world
        .audio
        .sync(frame(Phase::Loading, 0.0, 9.0, Some("ws1")))
        .unwrap();
    assert_eq!(
        world.audio.world_sound_count(),
        0,
        "nothing plays while loading"
    );
    render(&mut world.audio, SAMPLE_RATE, MENU_FADE * 3);
    assert_eq!(world.audio.music_state(), Some(PlaybackState::Stopped));

    world
        .audio
        .sync(frame(Phase::Revealing, 0.0, 9.0, Some("ws1")))
        .unwrap();
    assert_eq!(world.audio.world_sound_count(), 1);
}

#[test]
fn returning_to_the_menu_fades_the_world_music_out_and_starts_the_menu_music() {
    let mut world = world_with(two_types(0.1, 0.1));
    in_world(&mut world.audio, "ws1");

    world.audio.sync(Phase::MainMenu).unwrap();

    assert_eq!(world.audio.world_sound_count(), 0);
    assert_eq!(world.audio.retiring_music_count(), 1);
    render(&mut world.audio, SAMPLE_RATE, MENU_FADE);
    assert_eq!(world.audio.music_state(), Some(PlaybackState::Playing));

    in_world(&mut world.audio, "ws1");
    assert_eq!(
        world.audio.world_sound_count(),
        1,
        "the place is resolved afresh after the menu"
    );
}

#[test]
fn a_direct_start_plays_the_world_music_and_not_the_menu_music() {
    let mut world = world_with(two_types(0.1, 0.1));
    world
        .audio
        .sync(frame(Phase::Loading, 0.0, 9.0, None))
        .unwrap();

    world
        .audio
        .sync(frame(Phase::Revealing, 0.0, 9.0, Some("ws1")))
        .unwrap();

    assert_eq!(world.audio.world_sound_count(), 1);
    assert_eq!(world.audio.music_state(), None);
}

fn settled_peak(mut catalog: MusicCatalog, edit_gain: impl FnOnce(&mut MusicCatalog)) -> f32 {
    edit_gain(&mut catalog);
    let mut world = world_with(catalog);
    in_world(&mut world.audio, "ws1");
    let mut peak = 0.0_f32;
    wait_until("the world music to become audible", || {
        render_buffer(&mut world.audio);
        peak = world.peak.take();
        peak > 0.0
    });
    for _ in 0..SETTLED_BUFFERS {
        render_buffer(&mut world.audio);
        peak = peak.max(world.peak.take());
    }
    peak
}

#[test]
fn the_world_music_is_scaled_by_the_category_chain_alone() {
    let mut catalog = two_types(0.0, 0.0);
    catalog.music_types[0].fade_seconds = 0.0;
    let full = settled_peak(catalog.clone(), |catalog| {
        for category in &mut catalog.sound_categories {
            category.static_multiplier = Some(1.0);
        }
    });
    let chained = settled_peak(catalog, |_| {});

    let expected = MASTER_CATEGORY_MULTIPLIER * MUSIC_CATEGORY_MULTIPLIER;
    assert!(
        (chained / full / expected - 1.0).abs() < 5e-3,
        "ratio {}, expected {expected}",
        chained / full
    );
}

#[test]
fn a_world_music_file_that_vanishes_before_its_plan_starts_is_fatal() {
    let mut world = world_with(two_types(0.1, 0.1));
    std::fs::remove_file(world.root.path().join("audio/music/a.flac")).unwrap();

    let error = world
        .audio
        .sync(frame(Phase::InGame, 0.0, 9.0, Some("ws1")))
        .unwrap_err();

    assert!(matches!(error, AudioError::MusicFile { .. }), "{error}");
}

#[test]
fn a_world_music_file_that_fails_to_decode_while_playing_is_fatal() {
    let mut world = world_with(two_types(0.1, 0.1));
    write_audio_file(
        world.root.path(),
        "music/a.flac",
        &undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
    );
    in_world(&mut world.audio, "ws1");

    let mut result = Ok(());
    wait_until("the decode error to surface", || {
        render_buffer(&mut world.audio);
        result = world.audio.sync(frame(Phase::InGame, 0.0, 9.0, None));
        result.is_err()
    });

    assert!(matches!(result, Err(AudioError::MusicStream { .. })));
}

fn playlist_catalog(layers: &[&[&str]], duration: f32) -> MusicCatalog {
    world_catalog(
        &[("ws1", "P"), ("ws2", "B")],
        vec![
            typed("P", 0, 0.1, &["pal"]),
            typed("B", 0, 0.1, &["music/b.flac"]),
        ],
        vec![
            palette("pal", layers, duration, Some(0.2)),
            silent("pause", 1.0),
            single("music/a1.flac", vec![]),
            single("music/a2.flac", vec![]),
            single("music/c.flac", vec![]),
            single("music/b.flac", vec![]),
        ],
    )
}

fn playing_files(world: &World) -> Vec<String> {
    let mut files = world.audio.world_sound_file_names();
    files.sort();
    files
}

fn wait_until_no_sound_plays(world: &mut World) {
    wait_until("the sounds that play to end", || {
        render_buffer(&mut world.audio);
        tick(&mut world.audio, 0.0, 9.0);
        world.audio.world_sound_count() == 0
    });
}

#[test]
fn a_palette_layer_plays_its_sub_tracks_one_after_another_across_a_timed_gap() {
    let mut world = world_with(playlist_catalog(
        &[
            &["music/a1.flac", "pause", "music/a2.flac"],
            &["music/c.flac"],
        ],
        20.0,
    ));
    in_world(&mut world.audio, "ws1");
    assert_eq!(
        playing_files(&world),
        ["a1.flac", "c.flac"],
        "the first sub-track of each layer"
    );

    wait_until_no_sound_plays(&mut world);
    tick(&mut world.audio, 0.9, 9.0);
    assert_eq!(
        world.audio.world_sound_count(),
        0,
        "the gap is over too early"
    );
    tick(&mut world.audio, 0.2, 9.0);
    assert_eq!(
        playing_files(&world),
        ["a2.flac"],
        "the second sub-track did not start"
    );

    wait_until_no_sound_plays(&mut world);
    assert!(
        matches!(plan(&world.audio), Some(PlayPlan::Palette { .. })),
        "the palette window is still open"
    );
    tick(&mut world.audio, 20.0, 9.0);
    assert_eq!(plan(&world.audio), None);
}

#[test]
fn a_palette_stops_a_layer_that_is_still_playing_when_its_duration_is_over() {
    let mut world = world_with(playlist_catalog(
        &[&["music/a1.flac", "music/a2.flac"]],
        0.3,
    ));
    in_world(&mut world.audio, "ws1");
    assert_eq!(world.audio.world_sound_count(), 1);

    tick(&mut world.audio, 0.4, 9.0);

    assert_eq!(world.audio.world_sound_count(), 0);
    assert_eq!(world.audio.retiring_music_count(), 1);
}

#[test]
fn a_waiting_type_change_starts_when_the_playing_sub_track_ends_without_the_next_one() {
    let mut world = world_with(playlist_catalog(
        &[&["music/a1.flac", "music/a2.flac"]],
        20.0,
    ));
    in_world(&mut world.audio, "ws1");

    in_world(&mut world.audio, "ws2");
    assert_eq!(playing_files(&world), ["a1.flac"]);
    assert_eq!(
        world.audio.retiring_music_count(),
        0,
        "the sub-track was cut"
    );

    wait_for_next_plan(&mut world);

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
    assert_eq!(
        playing_files(&world),
        ["b.flac"],
        "the next sub-track of the old palette started"
    );
}

#[test]
fn a_waiting_type_change_starts_when_the_palette_window_ends_first() {
    let mut world = world_with(playlist_catalog(
        &[&["music/a1.flac", "music/a2.flac"]],
        0.3,
    ));
    in_world(&mut world.audio, "ws1");
    in_world(&mut world.audio, "ws2");
    assert_eq!(playing_files(&world), ["a1.flac"]);

    tick(&mut world.audio, 0.4, 9.0);

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
    assert_eq!(playing_files(&world), ["b.flac"]);
    assert_eq!(
        world.audio.retiring_music_count(),
        1,
        "the palette's sub-track fades out over the palette's fade"
    );
}

#[test]
fn a_type_change_queued_while_one_layer_plays_and_another_waits_in_a_gap_starts_no_gap_sub_track() {
    let mut world = world_with(playlist_catalog(
        &[&["music/a1.flac"], &["pause", "music/c.flac"]],
        20.0,
    ));
    in_world(&mut world.audio, "ws1");
    assert_eq!(playing_files(&world), ["a1.flac"]);

    in_world(&mut world.audio, "ws2");
    tick(&mut world.audio, 1.1, 9.0);
    assert_eq!(
        playing_files(&world),
        ["a1.flac"],
        "the gap ended while the change waited and its sub-track started"
    );
    assert_eq!(world.audio.retiring_music_count(), 0);

    wait_for_next_plan(&mut world);

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
    assert_eq!(playing_files(&world), ["b.flac"]);
}

#[test]
fn a_waiting_type_change_does_not_wait_through_a_gap_between_sub_tracks() {
    let mut world = world_with(playlist_catalog(
        &[&["music/a1.flac", "pause", "music/a2.flac"]],
        20.0,
    ));
    in_world(&mut world.audio, "ws1");
    in_world(&mut world.audio, "ws2");

    wait_for_next_plan(&mut world);

    assert_eq!(plan(&world.audio), Some(single_plan("music/b.flac")));
}
