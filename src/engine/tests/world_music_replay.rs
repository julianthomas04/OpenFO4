//! A headless replay of the world music along a walk: the real catalog and converted music files,
//! driven by simulated time through kira's mock backend, with the trace the world music emits.
//!
//! Ignored by default (needs an installation converted with the music tables and audio). Run with
//! `cargo test --test world_music_replay -- --ignored --nocapture`.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::streaming::StreamingSoundData;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::music::MusicCatalog;
use openfo4::assets::layout::audio_dir;
use openfo4::audio::conditions::GameTime;
use openfo4::audio::place::PlaceSnapshot;
use openfo4::audio::scheduler::{LayerStep, PlayPlan};
use openfo4::audio::test_support::{
    MockAudio, PeakReader, SharedBuffer, mock_manager_settings, render_buffer, wait_until,
};
use openfo4::audio::world_music::WorldMusic;
use openfo4::audio::{
    AudioFrame, AudioSystem, MenuMusicSettings, PLACE_POLLS_PER_SECOND, PlacePoll,
};
use openfo4::bethesda_space::UNITS_TO_ENGINE;
use openfo4::game_flow::Phase;

const COMMONWEALTH: &str = "00003C:Fallout4.esm";
const RED_ROCKET: &str = "Red Rocket Truck Stop";
const SANCTUARY: &str = "Sanctuary";
const FLY_TARGETS: [&str; 6] = [
    "Diamond City",
    "Goodneighbor",
    "Bunker Hill",
    "Lexington",
    "Covenant",
    "Cambridge Police Station",
];
const FLY_TARGET_COUNT: usize = 3;
const CELL_SIZE: f32 = 4096.0;
const WALK_SPEED_ENGINE_UNITS: f32 = 5.0;
const FRAME_SECONDS: f32 = 1.0 / 60.0;
const MOCK_SAMPLE_RATE: u32 = 7680;
const HOUR: f32 = 9.783;
const STAND_SECONDS: f32 = 180.0;
const FLY_STOP_SECONDS: f32 = 90.0;
const AUDIBLE_PEAK: f32 = 0.0001;
const MIN_REPORTED_SILENCE: f32 = 1.0;
const SEED: u64 = 5;

fn place_at(point: (f32, f32)) -> PlaceSnapshot {
    PlaceSnapshot {
        worldspace: COMMONWEALTH.to_string(),
        interior_cell: None,
        grid: (
            (point.0 / CELL_SIZE).floor() as i32,
            (point.1 / CELL_SIZE).floor() as i32,
        ),
        point_beth: point,
    }
}

#[derive(Debug)]
struct Silence {
    start: f32,
    end: f32,
    music_types: Vec<String>,
}

struct Replay {
    audio: MockAudio,
    meter: PeakReader,
    poll: PlacePoll,
    trace: SharedBuffer,
    clock: f32,
    position: (f32, f32),
    silent_since: Option<f32>,
    silent_types: Vec<String>,
    silences: Vec<Silence>,
    audible_seconds: f32,
    measured_seconds: f32,
    longest_silence: f32,
    plans: Vec<(f32, Option<PlayPlan>)>,
    log: Vec<String>,
}

impl Replay {
    fn new(catalog: MusicCatalog, install: &std::path::Path, trace: SharedBuffer) -> Self {
        let audio_root = audio_dir(install.to_str().unwrap());
        let (manager_settings, meter) = mock_manager_settings(MOCK_SAMPLE_RATE);
        let settings = MenuMusicSettings {
            enabled: false,
            file: "\\Data\\Music\\Special\\MUS_MainTheme.xwm".to_string(),
            fade: Duration::from_millis(100),
            attenuation_millibels: 0,
        };
        let mut audio = AudioSystem::new(manager_settings, install, settings).unwrap();
        let music = WorldMusic::new(catalog, std::path::Path::new(&audio_root), SEED).unwrap();
        audio.set_world_music(music);
        Self {
            audio,
            meter,
            poll: PlacePoll::new(PLACE_POLLS_PER_SECOND),
            trace,
            clock: 0.0,
            position: (0.0, 0.0),
            silent_since: None,
            silent_types: Vec::new(),
            silences: Vec::new(),
            audible_seconds: 0.0,
            measured_seconds: 0.0,
            longest_silence: 0.0,
            plans: Vec::new(),
            log: Vec::new(),
        }
    }

    fn current_type(&self) -> String {
        self.audio
            .world_music()
            .and_then(WorldMusic::current_music_type)
            .and_then(|music_type| music_type.editor_id.clone())
            .unwrap_or_else(|| "(none)".to_string())
    }

    fn frame(&mut self, point: (f32, f32)) {
        self.position = point;
        let dt = Duration::from_secs_f32(FRAME_SECONDS);
        let place = self.poll.due(dt).then(|| place_at(point));
        self.audio
            .sync(AudioFrame {
                phase: Phase::InGame,
                dt,
                time: GameTime::at_hour(HOUR),
                place,
            })
            .unwrap();
        render_buffer(&mut self.audio);
        std::thread::sleep(Duration::from_micros(150));
        self.clock += FRAME_SECONDS;
        self.record_plan();
        self.print_trace();
        self.track_silence();
    }

    fn record_plan(&mut self) {
        let plan = self
            .audio
            .world_music()
            .and_then(|music| music.current_plan().cloned());
        if self.plans.last().map(|(_, last)| last) != Some(&plan) {
            self.plans.push((self.clock, plan));
        }
    }

    /// The plans that can account for a silence: the one decided last before it began and every
    /// one decided during it.
    fn plans_explaining(&self, silence: &Silence) -> impl Iterator<Item = &PlayPlan> {
        let before = self
            .plans
            .iter()
            .rposition(|(time, _)| *time <= silence.start)
            .unwrap_or(0);
        self.plans[before..]
            .iter()
            .take_while(|(time, _)| *time <= silence.end)
            .filter_map(|(_, plan)| plan.as_ref())
    }

    fn print_trace(&mut self) {
        let lines = self.trace.drain();
        if lines.is_empty() {
            return;
        }
        println!(
            "[t={:7.2}s at ({:.0}, {:.0}) type {}]",
            self.clock,
            self.position.0,
            self.position.1,
            self.current_type()
        );
        for line in lines.lines() {
            println!("    {line}");
            self.log.push(line.to_string());
        }
    }

    fn track_silence(&mut self) {
        let audible = self.meter.take() > AUDIBLE_PEAK;
        self.measured_seconds += FRAME_SECONDS;
        if audible {
            self.audible_seconds += FRAME_SECONDS;
        }
        let current = self.current_type();
        match (audible, self.silent_since) {
            (false, None) => {
                self.silent_since = Some(self.clock);
                self.silent_types = vec![current];
            }
            (false, Some(_)) => {
                if self.silent_types.last() != Some(&current) {
                    self.silent_types.push(current);
                }
            }
            (true, Some(start)) => {
                self.silent_since = None;
                self.longest_silence = self.longest_silence.max(self.clock - start);
                if self.clock - start >= MIN_REPORTED_SILENCE {
                    self.silences.push(Silence {
                        start,
                        end: self.clock,
                        music_types: std::mem::take(&mut self.silent_types),
                    });
                }
            }
            (true, None) => {}
        }
    }

    fn walk(&mut self, from: (f32, f32), to: (f32, f32)) {
        let step = WALK_SPEED_ENGINE_UNITS / UNITS_TO_ENGINE * FRAME_SECONDS;
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx.hypot(dy);
        let frames = (length / step).ceil() as u32;
        for index in 1..=frames {
            let along = (index as f32 * step / length).min(1.0);
            self.frame((from.0 + dx * along, from.1 + dy * along));
        }
    }

    fn stand(&mut self, point: (f32, f32), seconds: f32) {
        for _ in 0..(seconds / FRAME_SECONDS) as u32 {
            self.frame(point);
        }
    }

    fn wait_for_first_sound(&mut self, point: (f32, f32)) {
        self.frame(point);
        wait_until("the first world sound to become audible", || {
            self.frame(point);
            self.silent_since.is_none()
        });
        self.silent_types.clear();
        self.silences.clear();
        self.audible_seconds = 0.0;
        self.measured_seconds = 0.0;
        self.longest_silence = 0.0;
    }
}

fn marker_position(db: &DbClient, name: &str) -> Option<(f32, f32)> {
    db.get_map_markers(COMMONWEALTH)
        .unwrap()
        .iter()
        .find(|marker| marker.name.as_deref() == Some(name))
        .map(|marker| (marker.position[0] as f32, marker.position[1] as f32))
}

fn type_named<'a>(
    catalog: &'a MusicCatalog,
    editor_id: &str,
) -> Option<&'a openfo4::assets::db::music::MusicType> {
    catalog
        .music_types
        .iter()
        .find(|music_type| music_type.editor_id.as_deref() == Some(editor_id))
}

fn file_seconds(audio_root: &Path, file_key: &str) -> f32 {
    StreamingSoundData::from_file(audio_root.join(file_key))
        .unwrap_or_else(|error| panic!("cannot open {file_key}: {error}"))
        .duration()
        .as_secs_f32()
}

/// The seconds of the intervals, counted once where they overlap.
fn union_length(mut intervals: Vec<(f32, f32)>) -> f32 {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut covered_to = 0.0_f32;
    for (start, end) in intervals {
        let start = start.max(covered_to);
        if end > start {
            total += end - start;
            covered_to = end;
        }
    }
    total
}

/// How long the plan can keep the music quiet: a silent plan, or the part of a palette's window
/// in which no layer's sound plays, by the lengths of the converted files.
fn quiet_seconds(plan: &PlayPlan, audio_root: &Path) -> f32 {
    match plan {
        PlayPlan::Single { .. } => 0.0,
        PlayPlan::Silent { seconds } => *seconds,
        PlayPlan::Palette {
            layers, duration, ..
        } => {
            let mut sounding = Vec::new();
            for layer in layers {
                let mut at = 0.0;
                for step in layer {
                    match step {
                        LayerStep::Sound { file_key } => {
                            let end = at + file_seconds(audio_root, file_key);
                            sounding.push((at, end.min(*duration)));
                            at = end;
                        }
                        LayerStep::Gap { seconds } => at += seconds,
                    }
                }
            }
            duration - union_length(sounding)
        }
    }
}

#[test]
#[ignore = "needs a converted installation with music tables and audio"]
fn walking_between_red_rocket_and_sanctuary_neither_cuts_a_track_nor_leaves_the_music_silent() {
    let trace = SharedBuffer::default();
    let _capture = trace.capture("openfo4::audio::world=debug");
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    let catalog = db.get_music_catalog().unwrap();
    let red_rocket = marker_position(&db, RED_ROCKET).expect("Red Rocket marker");
    let sanctuary = marker_position(&db, SANCTUARY).expect("Sanctuary marker");
    let fly_points: Vec<(f32, f32)> = FLY_TARGETS
        .iter()
        .filter_map(|name| marker_position(&db, name))
        .take(FLY_TARGET_COUNT)
        .collect();
    assert_eq!(fly_points.len(), FLY_TARGET_COUNT);

    let audio_root = PathBuf::from(audio_dir(common::asset_root().to_str().unwrap()));
    let mut replay = Replay::new(catalog.clone(), &common::asset_root(), trace);
    replay.wait_for_first_sound(red_rocket);
    replay.walk(red_rocket, sanctuary);
    replay.stand(sanctuary, 5.0);
    replay.walk(sanctuary, red_rocket);
    replay.stand(red_rocket, STAND_SECONDS);
    for point in fly_points {
        replay.stand(point, FLY_STOP_SECONDS);
    }

    println!(
        "audible {:.1} s of {:.1} s ({:.1} %), longest silence {:.1} s",
        replay.audible_seconds,
        replay.measured_seconds,
        100.0 * replay.audible_seconds / replay.measured_seconds,
        replay.longest_silence
    );
    println!("silences of at least {MIN_REPORTED_SILENCE} s:");
    let mut violations = Vec::new();
    for silence in &replay.silences {
        let quiet: f32 = replay
            .plans_explaining(silence)
            .map(|plan| quiet_seconds(plan, &audio_root))
            .sum();
        let fades: f32 = silence
            .music_types
            .iter()
            .filter_map(|name| type_named(&catalog, name))
            .map(|music_type| music_type.fade_seconds)
            .sum();
        let allowed = quiet + fades;
        let length = silence.end - silence.start;
        println!(
            "  {:.1}s..{:.1}s ({length:.1} s, allowed {allowed:.1} s) during {:?}",
            silence.start, silence.end, silence.music_types
        );
        if length > allowed {
            violations.push(format!(
                "{:.1}s..{:.1}s silent for {length:.1} s, allowed {allowed:.1} s",
                silence.start, silence.end
            ));
        }
    }
    if let Some(start) = replay.silent_since {
        violations.push(format!(
            "silent from {start:.1}s to the end of the replay ({:.1}s)",
            replay.clock
        ));
    }
    let cut_tracks: Vec<&String> = replay
        .log
        .iter()
        .filter(|line| {
            line.contains("type change: fading out")
                && line.contains("audible=true")
                && !line.contains("abrupt=true")
        })
        .collect();
    assert!(
        cut_tracks.is_empty(),
        "a playing track was faded out: {cut_tracks:#?}"
    );
    assert!(violations.is_empty(), "{violations:#?}");
}
