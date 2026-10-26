//! The kira sub-track of the looping sounds: files that loop from memory, fade in and out, and
//! follow the gain they are asked for. `plan_loops` decides; this runs what it decides.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use kira::sound::static_sound::StaticSoundHandle;
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, PlaybackRate};

use super::backend::AudioBackend;
use super::decoded_files::DecodedFiles;
use super::error::AudioError;
use super::loop_plan::{LoopAction, LoopTarget, plan_loops};
use super::music_output::fade_tween;

/// How long a loop takes to fade in, to fade out and to move to a new gain.
pub const LOOP_FADE: Duration = Duration::from_millis(1500);

struct Playing {
    handle: StaticSoundHandle,
    /// The gain the loop was last given; a change of it is a tween towards the new gain.
    gain_db: f32,
}

pub struct LoopOutput {
    track: TrackHandle,
    files: DecodedFiles,
    /// The playing loops by file key. A loop that was stopped fades out on its own and is no
    /// longer held here.
    playing: BTreeMap<String, Playing>,
}

impl LoopOutput {
    pub fn new<B: AudioBackend>(
        manager: &mut AudioManager<B>,
        audio_dir: PathBuf,
    ) -> Result<Self, AudioError> {
        Ok(Self {
            track: manager
                .add_sub_track(TrackBuilder::new())
                .map_err(AudioError::LoopTrack)?,
            files: DecodedFiles::new(audio_dir),
            playing: BTreeMap::new(),
        })
    }

    /// Brings the playing loops to the wanted ones, as `plan_loops` plans it: new loops fade in at
    /// their pitch, loops that are no longer wanted fade out and the others move to their new
    /// gain. The first start of a file decodes it. A file that cannot be decoded or a track that
    /// cannot take another sound is an error.
    pub fn follow(&mut self, targets: &[LoopTarget]) -> Result<(), AudioError> {
        let active: BTreeMap<String, f32> = self
            .playing
            .iter()
            .map(|(file_key, playing)| (file_key.clone(), playing.gain_db))
            .collect();
        for action in plan_loops(&active, targets) {
            match action {
                LoopAction::Start {
                    file_key,
                    gain_db,
                    playback_rate,
                } => self.start(file_key, gain_db, playback_rate)?,
                LoopAction::SetGain { file_key, gain_db } => self.set_gain(&file_key, gain_db),
                LoopAction::Stop { file_key } => self.stop(&file_key),
            }
        }
        Ok(())
    }

    fn start(
        &mut self,
        file_key: String,
        gain_db: f32,
        playback_rate: f64,
    ) -> Result<(), AudioError> {
        let data = self
            .files
            .get(&file_key)?
            .loop_region(..)
            .volume(Decibels(gain_db))
            .playback_rate(PlaybackRate(playback_rate))
            .fade_in_tween(fade_tween(LOOP_FADE));
        let handle = self
            .track
            .play(data)
            .map_err(|source| AudioError::LoopPlayback {
                path: self.files.path(&file_key),
                source,
            })?;
        self.playing.insert(file_key, Playing { handle, gain_db });
        Ok(())
    }

    fn set_gain(&mut self, file_key: &str, gain_db: f32) {
        if let Some(playing) = self.playing.get_mut(file_key) {
            playing
                .handle
                .set_volume(Decibels(gain_db), fade_tween(LOOP_FADE));
            playing.gain_db = gain_db;
        }
    }

    fn stop(&mut self, file_key: &str) {
        if let Some(mut playing) = self.playing.remove(file_key) {
            playing.handle.stop(fade_tween(LOOP_FADE));
        }
    }

    #[cfg(test)]
    pub fn playing_files(&self) -> Vec<&str> {
        self.playing.keys().map(String::as_str).collect()
    }

    #[cfg(test)]
    pub fn decoded_count(&self) -> usize {
        self.files.len()
    }

    #[cfg(test)]
    fn position(&self, file_key: &str) -> Option<f64> {
        self.playing
            .get(file_key)
            .map(|playing| playing.handle.position())
    }
}

#[cfg(test)]
mod tests {
    use kira::backend::mock::MockBackend;

    use super::*;
    use crate::audio::test_support::{
        HALF_SECOND_SAMPLES, PeakReader, SAMPLE_RATE, SETTLED_BUFFERS, mock_manager_settings,
        render_manager, render_manager_buffer, sine_flac, undecodable_flac, write_audio_file,
    };

    const WIND: &str = "sound/fx/amb/wind.flac";
    const RAIN: &str = "sound/fx/amb/rain.flac";
    /// Three loops of the half second files.
    const SEVERAL_LOOPS: Duration = Duration::from_millis(1600);
    const LOUDEST: f32 = 0.0;
    const HALF_AMPLITUDE_DB: f32 = -6.0206;
    const AS_IS: f64 = 1.0;

    struct Rig {
        root: tempfile::TempDir,
        manager: AudioManager<MockBackend>,
        output: LoopOutput,
        peak: PeakReader,
    }

    impl Rig {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            for key in [WIND, RAIN] {
                write_audio_file(
                    root.path(),
                    key,
                    &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
                );
            }
            let (settings, peak) = mock_manager_settings(SAMPLE_RATE);
            let mut manager = AudioManager::new(settings).unwrap();
            let output = LoopOutput::new(&mut manager, root.path().join("audio")).unwrap();
            Self {
                root,
                manager,
                output,
                peak,
            }
        }

        fn follow(&mut self, targets: &[(&str, f32)]) -> Result<(), AudioError> {
            let targets: Vec<LoopTarget> = targets
                .iter()
                .map(|&(file_key, gain_db)| LoopTarget {
                    file_key: file_key.to_string(),
                    gain_db,
                    playback_rate: AS_IS,
                })
                .collect();
            self.output.follow(&targets)
        }

        fn render(&mut self, duration: Duration) {
            render_manager(&mut self.manager, SAMPLE_RATE, duration);
        }

        /// The loudest sample of a few buffers once the fade of the commands so far is over.
        fn peak_after_settling(&mut self) -> f32 {
            self.render(LOOP_FADE);
            self.peak.take();
            for _ in 0..SETTLED_BUFFERS {
                render_manager_buffer(&mut self.manager);
            }
            self.peak.take()
        }
    }

    #[test]
    fn a_started_loop_keeps_playing_past_the_end_of_its_file() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]).unwrap();

        rig.render(SEVERAL_LOOPS);

        assert!(rig.peak_after_settling() > 0.0);
        assert_eq!(rig.output.playing_files(), [WIND]);
    }

    #[test]
    fn a_loop_plays_at_the_gain_it_starts_with() {
        let peak_at = |gain_db: f32| {
            let mut rig = Rig::new();
            rig.follow(&[(WIND, gain_db)]).unwrap();
            rig.peak_after_settling()
        };

        let loud = peak_at(LOUDEST);
        let half = peak_at(HALF_AMPLITUDE_DB);

        assert!(loud > 0.0);
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
    }

    #[test]
    fn a_playing_loop_moves_to_the_gain_it_is_asked_for() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]).unwrap();
        let loud = rig.peak_after_settling();

        rig.follow(&[(WIND, HALF_AMPLITUDE_DB)]).unwrap();
        let half = rig.peak_after_settling();

        assert!(loud > 0.0);
        assert!((half / loud - 0.5).abs() < 0.02, "{half} against {loud}");
        assert_eq!(rig.output.playing_files(), [WIND], "one loop, not two");
    }

    #[test]
    fn a_loop_fades_in_from_silence() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]).unwrap();
        render_manager_buffer(&mut rig.manager);
        render_manager_buffer(&mut rig.manager);
        let early = rig.peak.take();

        let settled = rig.peak_after_settling();

        assert!(settled > 0.0);
        assert!(early < settled / 10.0, "{early} is not far below {settled}");
    }

    #[test]
    fn a_loop_that_is_no_longer_wanted_fades_out_to_silence_and_is_dropped() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST)]).unwrap();
        assert!(rig.peak_after_settling() > 0.0);

        rig.follow(&[]).unwrap();
        rig.render(LOOP_FADE);
        rig.peak.take();
        rig.render(Duration::from_millis(100));

        assert_eq!(rig.peak.take(), 0.0);
        assert_eq!(rig.output.playing_files(), Vec::<&str>::new());
    }

    #[test]
    fn several_loops_follow_their_own_targets() {
        let mut rig = Rig::new();
        rig.follow(&[(WIND, LOUDEST), (RAIN, LOUDEST)]).unwrap();
        assert_eq!(rig.output.playing_files(), [RAIN, WIND]);

        rig.follow(&[(RAIN, LOUDEST)]).unwrap();

        assert_eq!(rig.output.playing_files(), [RAIN]);
    }

    #[test]
    fn a_file_is_decoded_once_however_often_its_loop_starts() {
        let mut rig = Rig::new();

        for _ in 0..3 {
            rig.follow(&[(WIND, LOUDEST)]).unwrap();
            rig.follow(&[]).unwrap();
        }

        assert_eq!(rig.output.decoded_count(), 1);
    }

    #[test]
    fn a_loop_plays_at_the_pitch_it_starts_with() {
        let mut slow = Rig::new();
        let mut fast = Rig::new();
        for (rig, playback_rate) in [(&mut slow, AS_IS), (&mut fast, 2.0)] {
            rig.output
                .follow(&[LoopTarget {
                    file_key: WIND.to_string(),
                    gain_db: LOUDEST,
                    playback_rate,
                }])
                .unwrap();
            rig.render(Duration::from_millis(200));
        }

        let slow_position = slow.output.position(WIND).unwrap();
        let fast_position = fast.output.position(WIND).unwrap();

        assert!(
            (fast_position / slow_position - 2.0).abs() < 0.1,
            "{fast_position} against {slow_position}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_decoded_or_is_missing_is_an_error_and_plays_nothing() {
        let mut rig = Rig::new();
        write_audio_file(
            rig.root.path(),
            WIND,
            &undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );

        for file in [WIND, "sound/fx/amb/absent.flac"] {
            let error = rig.follow(&[(file, LOUDEST)]).unwrap_err();
            assert!(matches!(error, AudioError::SfxFile { .. }), "{error}");
        }

        assert_eq!(rig.output.playing_files(), Vec::<&str>::new());
    }
}
