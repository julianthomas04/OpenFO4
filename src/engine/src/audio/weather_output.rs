//! The kira sub-track of the weather sounds: loops that stream from their files, fade in and out
//! and follow the gain they are asked for, and the thunder that plays once. `plan_loops` decides
//! what the loops do; this runs it.
//!
//! No weather sound is worth stopping the game for: a file that cannot be opened, played or
//! decoded is logged once and left out from then on, and the other sounds play on.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, PlaySoundError, PlaybackRate};

use super::backend::AudioBackend;
use super::error::AudioError;
use super::loop_plan::{LoopAction, LoopTarget, plan_loops};
use super::music_output::{fade_tween, open_stream};
use super::sound_resolve::LayerPlay;
use super::weather::LOG_TARGET;

/// How long a loop takes to fade in, to fade out and to move to a new gain.
pub const LOOP_FADE: Duration = Duration::from_millis(1500);

type Data = StreamingSoundData<FromFileError>;

#[derive(Debug, thiserror::Error)]
enum SoundFailure {
    #[error(
        "{path} cannot be opened; the installer converts it from the game, \
         so run the installer again: {source}"
    )]
    Open {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("{path} cannot be played: {source}")]
    Play {
        path: PathBuf,
        source: PlaySoundError<FromFileError>,
    },
    #[error(
        "{path} failed to decode while playing; the file is damaged, \
         so run the installer again: {source}"
    )]
    Decode {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("{path} stopped although it should loop")]
    Stopped { path: PathBuf },
}

enum Progress {
    Playing,
    Ended,
    Failed(SoundFailure),
}

struct Stream {
    file_key: String,
    path: PathBuf,
    handle: StreamingSoundHandle<FromFileError>,
}

impl Stream {
    /// A sound is marked stopped only after its decode error is queued, so reading the state
    /// before the error never misses one.
    fn progress(&mut self) -> Progress {
        let stopped = self.handle.state() == PlaybackState::Stopped;
        match self.handle.pop_error() {
            Some(source) => Progress::Failed(SoundFailure::Decode {
                path: self.path.clone(),
                source,
            }),
            None if stopped => Progress::Ended,
            None => Progress::Playing,
        }
    }

    fn stopped_early(&self) -> SoundFailure {
        SoundFailure::Stopped {
            path: self.path.clone(),
        }
    }
}

struct PlayingLoop {
    stream: Stream,
    /// The gain the loop was last given; a change of it is a tween towards the new gain.
    gain_db: f32,
}

pub struct WeatherOutput {
    track: TrackHandle,
    audio_dir: PathBuf,
    /// The loops that are wanted, by file key.
    loops: BTreeMap<String, PlayingLoop>,
    /// The sounds that take no more commands but are watched until they stop: loops fading out
    /// and thunder.
    finishing: Vec<Stream>,
    /// The files that failed; they are left out from then on.
    failed: HashSet<String>,
}

impl WeatherOutput {
    pub fn new<B: AudioBackend>(
        manager: &mut AudioManager<B>,
        audio_dir: PathBuf,
    ) -> Result<Self, AudioError> {
        Ok(Self {
            track: manager
                .add_sub_track(TrackBuilder::new())
                .map_err(AudioError::WeatherTrack)?,
            audio_dir,
            loops: BTreeMap::new(),
            finishing: Vec::new(),
            failed: HashSet::new(),
        })
    }

    /// Brings the playing loops to the wanted ones, as `plan_loops` plans it: new loops stream in
    /// at their pitch and fade in, loops that are no longer wanted fade out and the others move to
    /// their new gain. A file that failed is not wanted.
    pub fn follow(&mut self, targets: &[LoopTarget]) {
        let wanted: Vec<LoopTarget> = targets
            .iter()
            .filter(|target| !self.failed.contains(&target.file_key))
            .cloned()
            .collect();
        let active: BTreeMap<String, f32> = self
            .loops
            .iter()
            .map(|(file_key, playing)| (file_key.clone(), playing.gain_db))
            .collect();
        for action in plan_loops(&active, &wanted) {
            match action {
                LoopAction::Start {
                    file_key,
                    gain_db,
                    playback_rate,
                } => self.start(file_key, gain_db, playback_rate),
                LoopAction::SetGain { file_key, gain_db } => self.set_gain(&file_key, gain_db),
                LoopAction::Stop { file_key } => self.stop(&file_key),
            }
        }
    }

    /// Starts the thunder layers, each at its own pitch and gain. A file that failed is skipped.
    pub fn play_thunder(&mut self, layers: &[LayerPlay]) {
        for layer in layers {
            if self.failed.contains(&layer.file_key) {
                continue;
            }
            match self.play(&layer.file_key, |path| thunder_data(path, layer)) {
                Ok(stream) => self.finishing.push(stream),
                Err(failure) => self.fail(&layer.file_key, &failure),
            }
        }
    }

    /// Looks after the sounds: one that failed, or that stopped although it should loop, is
    /// dropped and its file is left out from then on, and one that has ended is let go.
    pub fn check(&mut self) {
        let mut failures = Vec::new();
        self.loops
            .retain(|file_key, playing| match playing.stream.progress() {
                Progress::Playing => true,
                Progress::Ended => {
                    failures.push((file_key.clone(), playing.stream.stopped_early()));
                    false
                }
                Progress::Failed(failure) => {
                    failures.push((file_key.clone(), failure));
                    false
                }
            });
        self.finishing.retain_mut(|stream| match stream.progress() {
            Progress::Playing => true,
            Progress::Ended => false,
            Progress::Failed(failure) => {
                failures.push((stream.file_key.clone(), failure));
                false
            }
        });
        for (file_key, failure) in failures {
            self.fail(&file_key, &failure);
        }
    }

    fn start(&mut self, file_key: String, gain_db: f32, playback_rate: f64) {
        match self.play(&file_key, |path| loop_data(path, gain_db, playback_rate)) {
            Ok(stream) => {
                self.loops.insert(file_key, PlayingLoop { stream, gain_db });
            }
            Err(failure) => self.fail(&file_key, &failure),
        }
    }

    fn set_gain(&mut self, file_key: &str, gain_db: f32) {
        if let Some(playing) = self.loops.get_mut(file_key) {
            playing
                .stream
                .handle
                .set_volume(Decibels(gain_db), fade_tween(LOOP_FADE));
            playing.gain_db = gain_db;
        }
    }

    fn stop(&mut self, file_key: &str) {
        if let Some(mut playing) = self.loops.remove(file_key) {
            playing.stream.handle.stop(fade_tween(LOOP_FADE));
            self.finishing.push(playing.stream);
        }
    }

    /// Opens the file `file_key` as `open` shapes it and starts it on the track.
    fn play(
        &mut self,
        file_key: &str,
        open: impl FnOnce(&Path) -> Result<Data, FromFileError>,
    ) -> Result<Stream, SoundFailure> {
        let path = self.audio_dir.join(file_key);
        let data = open(&path).map_err(|source| SoundFailure::Open {
            path: path.clone(),
            source,
        })?;
        let handle = self.track.play(data).map_err(|source| SoundFailure::Play {
            path: path.clone(),
            source,
        })?;
        Ok(Stream {
            file_key: file_key.to_string(),
            path,
            handle,
        })
    }

    /// Leaves the file out from now on and says why, the first time.
    fn fail(&mut self, file_key: &str, failure: &SoundFailure) {
        if self.failed.insert(file_key.to_string()) {
            tracing::error!(
                target: LOG_TARGET,
                "weather sound {failure}; it is left out from now on"
            );
        }
    }

    #[cfg(test)]
    pub fn playing_files(&self) -> Vec<&str> {
        self.loops.keys().map(String::as_str).collect()
    }

    /// The sounds that are playing or fading out.
    #[cfg(test)]
    pub fn sound_count(&self) -> usize {
        self.loops.len() + self.finishing.len()
    }
}

/// The whole file, repeated, fading in at its gain and pitch.
fn loop_data(path: &Path, gain_db: f32, playback_rate: f64) -> Result<Data, FromFileError> {
    Ok(open_stream(path, true)?
        .volume(Decibels(gain_db))
        .playback_rate(PlaybackRate(playback_rate))
        .fade_in_tween(fade_tween(LOOP_FADE)))
}

/// The file once, from its first sample, at the pitch and gain of the layer.
fn thunder_data(path: &Path, layer: &LayerPlay) -> Result<Data, FromFileError> {
    Ok(open_stream(path, false)?
        .volume(Decibels(layer.mix.gain_db))
        .playback_rate(PlaybackRate(layer.mix.playback_rate)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use kira::backend::mock::MockBackend;
    use kira::sound::streaming::{Decoder, StreamingSoundSettings};
    use kira::{Frame, Tween};

    use super::*;
    use crate::audio::sound_mix::LayerMix;
    use crate::audio::test_support::{
        HALF_SECOND_SAMPLES, PeakReader, SAMPLE_RATE, SharedBuffer, mock_manager_settings,
        peak_when_heard, render_manager, render_manager_paced, sine_flac, wait_until,
        write_audio_file,
    };

    const WIND: &str = "sound/fx/amb/wind.flac";
    const RAIN: &str = "sound/fx/amb/rain.flac";
    const BOOM: &str = "sound/fx/amb/boom.flac";
    const ABSENT: &str = "sound/fx/amb/absent.flac";
    const LOUDEST: f32 = 0.0;
    const HALF_AMPLITUDE_DB: f32 = -6.0206;
    const AS_IS: f64 = 1.0;
    /// Three loops of the half second files.
    const SEVERAL_LOOPS: Duration = Duration::from_millis(1600);
    /// The stretch of rendering a level is read over.
    const READING: Duration = Duration::from_millis(100);
    /// The stretch of rendering over which the positions of two loops are compared.
    const COMPARING: Duration = Duration::from_millis(50);
    /// The least that is worth rendering between two looks at the sounds.
    const STEP: Duration = Duration::from_millis(16);
    const REPORT: &str = "left out from now on";

    /// A decoder that fails at once, as a damaged file does to the stream that plays it, and sets
    /// its flag when it does. The decode thread of a stream whose real file is damaged keeps
    /// spinning for the rest of the test run, so the tests use this one instead.
    struct Broken(Arc<AtomicBool>);

    impl Decoder for Broken {
        type Error = FromFileError;

        fn sample_rate(&self) -> u32 {
            SAMPLE_RATE
        }

        fn num_frames(&self) -> usize {
            HALF_SECOND_SAMPLES as usize
        }

        fn decode(&mut self) -> Result<Vec<Frame>, FromFileError> {
            self.0.store(true, Ordering::SeqCst);
            Err(FromFileError::NoDefaultTrack)
        }

        fn seek(&mut self, _index: usize) -> Result<usize, FromFileError> {
            Ok(0)
        }
    }

    fn install() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for key in [WIND, RAIN, BOOM] {
            write_audio_file(
                root.path(),
                key,
                &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
            );
        }
        root
    }

    fn layer(file_key: &str, gain_db: f32, playback_rate: f64) -> LayerPlay {
        LayerPlay {
            file_key: file_key.to_string(),
            mix: LayerMix {
                playback_rate,
                gain_db,
            },
        }
    }

    fn reports(logged: &str) -> Vec<&str> {
        logged
            .lines()
            .filter(|line| line.contains(REPORT))
            .collect()
    }

    struct Rig {
        _root: tempfile::TempDir,
        manager: AudioManager<MockBackend>,
        output: WeatherOutput,
        peak: PeakReader,
    }

    impl Rig {
        fn new() -> Self {
            let root = install();
            let (settings, peak) = mock_manager_settings(SAMPLE_RATE);
            let mut manager = AudioManager::new(settings).unwrap();
            let output = WeatherOutput::new(&mut manager, root.path().join("audio")).unwrap();
            Self {
                _root: root,
                manager,
                output,
                peak,
            }
        }

        fn follow(&mut self, targets: &[(&str, f32)]) {
            let targets: Vec<LoopTarget> = targets
                .iter()
                .map(|&(file_key, gain_db)| LoopTarget {
                    file_key: file_key.to_string(),
                    gain_db,
                    playback_rate: AS_IS,
                })
                .collect();
            self.output.follow(&targets);
        }

        fn render(&mut self, duration: Duration) {
            render_manager_paced(&mut self.manager, SAMPLE_RATE, duration);
        }

        /// The loudest sample of the next `READING` of rendering.
        fn peak_over_reading(&mut self) -> f32 {
            self.peak.take();
            self.render(READING);
            self.peak.take()
        }

        /// The loudest sample of the first `READING` of rendering that has anything to hear. A
        /// sound that has just started may not come through for a while, so use this where a
        /// sound is expected; where silence is, `peak_over_reading` does not wait.
        fn heard_over_reading(&mut self) -> f32 {
            peak_when_heard(&mut self.manager, &self.peak, SAMPLE_RATE, READING)
        }

        /// The loudest sample once the fade of the commands so far is over.
        fn heard_after_settling(&mut self) -> f32 {
            self.render(LOOP_FADE);
            self.heard_over_reading()
        }

        /// Lets the sounds run for a little and looks after them.
        fn notice_failures(&mut self) {
            self.render(READING);
            self.output.check();
        }

        /// Starts a sound whose decoder fails at once, for the file `file_key`, and waits until it
        /// has: the sound is not rendered before its failure is on its way.
        fn start_broken(&mut self, file_key: &str) -> Stream {
            let failed = Arc::new(AtomicBool::new(false));
            let data = StreamingSoundData::from_decoder(Broken(failed.clone()));
            let stream = self.output.play(file_key, |_| Ok(data)).unwrap();
            wait_until("the decoder to fail", || failed.load(Ordering::SeqCst));
            stream
        }

        /// Plays a loop of `file_key` whose decoder fails at once.
        fn start_broken_loop(&mut self, file_key: &str) {
            let stream = self.start_broken(file_key);
            self.output.loops.insert(
                file_key.to_string(),
                PlayingLoop {
                    stream,
                    gain_db: LOUDEST,
                },
            );
        }

        /// Plays a thunder layer of `file_key` whose decoder fails at once.
        fn start_broken_thunder(&mut self, file_key: &str) {
            let stream = self.start_broken(file_key);
            self.output.finishing.push(stream);
        }
    }

    /// Stops the sounds that are left, so their decode threads end with the test. A sound stops
    /// whether or not its stream has data, so the rendering is not paced.
    impl Drop for Rig {
        fn drop(&mut self) {
            if std::thread::panicking() {
                return;
            }
            self.output.follow(&[]);
            render_manager(&mut self.manager, SAMPLE_RATE, LOOP_FADE + READING);
        }
    }

    #[test]
    fn a_loop_streams_its_whole_file_over_and_over_at_its_gain_and_pitch_fading_in() {
        let root = install();
        let path = root.path().join("audio").join(WIND);

        let data = loop_data(&path, -6.0, 1.25).unwrap();

        assert_eq!(
            data.settings,
            StreamingSoundSettings::new()
                .loop_region(..)
                .volume(Decibels(-6.0))
                .playback_rate(PlaybackRate(1.25))
                .fade_in_tween(fade_tween(LOOP_FADE))
        );
    }

    #[test]
    fn a_thunder_layer_streams_its_file_once_at_its_gain_and_pitch_from_the_first_sample() {
        let root = install();
        let path = root.path().join("audio").join(BOOM);

        let data = thunder_data(&path, &layer(BOOM, -12.0, 0.9)).unwrap();

        assert_eq!(
            data.settings,
            StreamingSoundSettings::new()
                .volume(Decibels(-12.0))
                .playback_rate(PlaybackRate(0.9))
        );
    }

    #[test]
    fn a_started_loop_keeps_playing_past_the_end_of_its_file() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]);

        rig.render(SEVERAL_LOOPS);
        rig.output.check();

        assert!(rig.heard_after_settling() > 0.0);
        assert_eq!(rig.output.playing_files(), [WIND]);
    }

    #[test]
    fn a_loop_plays_at_the_gain_it_starts_with() {
        let heard_at = |gain_db: f32| {
            let mut rig = Rig::new();
            rig.follow(&[(WIND, gain_db)]);
            rig.heard_after_settling()
        };

        let loud = heard_at(LOUDEST);
        let half = heard_at(HALF_AMPLITUDE_DB);

        assert!(loud > 0.0);
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
    }

    #[test]
    fn a_playing_loop_moves_to_the_gain_it_is_asked_for_over_the_loop_fade() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]);
        let loud = rig.heard_after_settling();

        rig.follow(&[(WIND, HALF_AMPLITUDE_DB)]);
        rig.render(LOOP_FADE / 3);
        let on_the_way = rig.heard_over_reading();
        let half = rig.heard_after_settling();

        assert!(loud > 0.0);
        assert!(
            on_the_way > loud * 0.65 && on_the_way < loud * 0.9,
            "{on_the_way} is not between {loud} and the half of it"
        );
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
        assert_eq!(rig.output.playing_files(), [WIND], "one loop, not two");
    }

    #[test]
    fn a_loop_fades_in_from_silence_over_the_loop_fade() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]);

        rig.render(LOOP_FADE / 2);
        let half_way = rig.peak_over_reading();
        rig.render(LOOP_FADE / 2);
        let faded_in = rig.heard_over_reading();
        let settled = rig.heard_after_settling();

        assert!(settled > 0.0);
        assert!(
            half_way < settled / 10.0,
            "{half_way} is not far below {settled}"
        );
        assert!(
            (faded_in / settled - 1.0).abs() < 0.02,
            "{faded_in} has not reached {settled}"
        );
    }

    #[test]
    fn a_loop_that_is_no_longer_wanted_fades_out_over_the_loop_fade_and_is_let_go() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]);
        let settled = rig.heard_after_settling();

        rig.follow(&[]);
        rig.render(LOOP_FADE / 2);
        rig.output.check();
        let still_fading = rig.output.sound_count();
        let half_way = rig.heard_over_reading();
        rig.render(LOOP_FADE / 2);
        rig.output.check();

        assert!(settled > 0.0);
        assert_eq!(still_fading, 1);
        assert!(half_way > 0.0 && half_way < settled / 10.0, "{half_way}");
        assert_eq!(rig.output.sound_count(), 0);
        assert_eq!(rig.peak_over_reading(), 0.0);
        assert_eq!(rig.output.playing_files(), Vec::<&str>::new());
    }

    #[test]
    fn several_loops_follow_their_own_targets() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST), (RAIN, LOUDEST)]);
        assert_eq!(rig.output.playing_files(), [RAIN, WIND]);

        rig.follow(&[(RAIN, LOUDEST)]);

        assert_eq!(rig.output.playing_files(), [RAIN]);
    }

    #[test]
    fn a_loop_plays_at_the_pitch_it_starts_with() {
        let mut rig = Rig::new();
        let at_rate = |file_key: &str, playback_rate: f64| LoopTarget {
            file_key: file_key.to_string(),
            gain_db: LOUDEST,
            playback_rate,
        };
        rig.output
            .follow(&[at_rate(WIND, AS_IS), at_rate(RAIN, 2.0)]);
        let position =
            |rig: &Rig, file_key: &str| rig.output.loops[file_key].stream.handle.position();

        wait_until("the faster loop to move twice as far as the slower", || {
            let (slow_from, fast_from) = (position(&rig, WIND), position(&rig, RAIN));
            rig.render(COMPARING);
            let slow = position(&rig, WIND) - slow_from;
            let fast = position(&rig, RAIN) - fast_from;
            slow > 0.0 && (fast / slow - 2.0).abs() < 0.1
        });
    }

    #[test]
    fn a_loop_file_that_cannot_be_opened_is_logged_once_and_left_out_while_the_others_play() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");

        for _ in 0..3 {
            rig.follow(&[(ABSENT, LOUDEST), (RAIN, LOUDEST)]);
        }

        assert_eq!(rig.output.playing_files(), [RAIN]);
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(logged.contains("absent.flac cannot be opened"), "{logged}");
        assert!(rig.heard_after_settling() > 0.0);
    }

    #[test]
    fn a_loop_that_fails_while_playing_is_logged_once_and_not_started_again() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let wanted = [(WIND, LOUDEST), (RAIN, LOUDEST)];
        rig.start_broken_loop(WIND);
        rig.follow(&wanted);

        wait_until("the failure to be noticed", || {
            rig.notice_failures();
            rig.output.playing_files() == [RAIN]
        });
        rig.follow(&wanted);
        rig.notice_failures();

        assert_eq!(rig.output.playing_files(), [RAIN]);
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(logged.contains("wind.flac failed to decode"), "{logged}");
        assert!(rig.heard_after_settling() > 0.0, "the rain plays on");
    }

    #[test]
    fn a_loop_that_stops_by_itself_is_logged_once_and_not_started_again() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        rig.follow(&[(WIND, LOUDEST)]);
        rig.output
            .loops
            .get_mut(WIND)
            .unwrap()
            .stream
            .handle
            .stop(Tween::default());

        rig.notice_failures();
        rig.follow(&[(WIND, LOUDEST)]);
        rig.notice_failures();

        assert_eq!(rig.output.playing_files(), Vec::<&str>::new());
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(
            logged.contains("wind.flac stopped although it should loop"),
            "{logged}"
        );
    }

    #[test]
    fn a_loop_that_fails_while_it_fades_out_is_left_out_too() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        rig.start_broken_loop(WIND);
        rig.follow(&[]);

        wait_until("the failure to be noticed", || {
            rig.notice_failures();
            rig.output.sound_count() == 0
        });
        rig.follow(&[(WIND, LOUDEST)]);

        assert_eq!(rig.output.playing_files(), Vec::<&str>::new());
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
    }

    #[test]
    fn a_thunder_layer_plays_at_its_gain_from_the_first_sample() {
        let heard_at = |gain_db: f32| {
            let mut rig = Rig::new();
            rig.output.play_thunder(&[layer(BOOM, gain_db, AS_IS)]);
            rig.heard_over_reading()
        };

        let loud = heard_at(LOUDEST);
        let half = heard_at(HALF_AMPLITUDE_DB);

        assert!(loud > 0.0);
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
    }

    #[test]
    fn a_thunder_layer_plays_once_and_is_let_go_when_it_ends() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        rig.output.play_thunder(&[layer(BOOM, LOUDEST, AS_IS)]);
        assert_eq!(rig.output.sound_count(), 1);

        wait_until("the thunder to end", || {
            rig.render(STEP);
            rig.output.check();
            rig.output.sound_count() == 0
        });

        assert_eq!(rig.peak_over_reading(), 0.0);
        assert_eq!(log.drain(), "");
    }

    #[test]
    fn the_layers_of_a_strike_play_together() {
        let mut rig = Rig::new();

        rig.output
            .play_thunder(&[layer(BOOM, LOUDEST, AS_IS), layer(RAIN, LOUDEST, AS_IS)]);

        assert_eq!(rig.output.sound_count(), 2);
        assert!(rig.heard_over_reading() > 0.0);
    }

    #[test]
    fn a_thunder_layer_whose_file_cannot_be_opened_is_logged_once_and_the_others_still_play() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let layers = [layer(ABSENT, LOUDEST, AS_IS), layer(BOOM, LOUDEST, AS_IS)];

        rig.output.play_thunder(&layers);
        rig.output.play_thunder(&layers);

        assert_eq!(rig.output.sound_count(), 2, "the boom, twice");
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(logged.contains("absent.flac cannot be opened"), "{logged}");
    }

    #[test]
    fn a_thunder_layer_that_fails_while_playing_is_logged_once_and_left_out_from_then_on() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        rig.start_broken_thunder(BOOM);

        wait_until("the failure to be noticed", || {
            rig.notice_failures();
            rig.output.sound_count() == 0
        });
        rig.output.play_thunder(&[layer(BOOM, LOUDEST, AS_IS)]);

        assert_eq!(rig.output.sound_count(), 0);
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(logged.contains("boom.flac failed to decode"), "{logged}");
    }

    #[test]
    fn a_file_that_fails_in_two_sounds_at_once_is_logged_once() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        rig.start_broken_thunder(BOOM);
        rig.start_broken_thunder(BOOM);

        wait_until("the failures to be noticed", || {
            rig.notice_failures();
            rig.output.sound_count() == 0
        });

        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
    }

    #[test]
    fn a_layer_the_track_has_no_room_for_is_logged_once_and_left_out() {
        let mut rig = Rig::new();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let capacity = rig.output.track.sound_capacity();

        rig.output
            .play_thunder(&vec![layer(BOOM, LOUDEST, AS_IS); capacity + 1]);

        assert_eq!(rig.output.sound_count(), capacity);
        let logged = log.drain();
        assert_eq!(reports(&logged).len(), 1, "{logged}");
        assert!(logged.contains("cannot be played"), "{logged}");
    }
}
