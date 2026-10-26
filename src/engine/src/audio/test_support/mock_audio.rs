use std::time::{Duration, Instant};

use kira::backend::mock::{MockBackend, MockBackendSettings};
use kira::track::MainTrackBuilder;
use kira::{AudioManager, AudioManagerSettings};

use super::meter::{PeakReader, peak_meter};
use crate::audio::AudioSystem;

pub type MockAudio = AudioSystem<MockBackend>;

const BUFFER_FRAMES: usize = 128;
const WAIT_STEP: Duration = Duration::from_millis(5);
const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// Manager settings for the mock backend with a peak meter on the main track, which carries
/// everything that plays.
pub fn mock_manager_settings(sample_rate: u32) -> (AudioManagerSettings<MockBackend>, PeakReader) {
    let (meter, reader) = peak_meter();
    let settings = AudioManagerSettings {
        backend_settings: MockBackendSettings { sample_rate },
        main_track_builder: MainTrackBuilder::new().with_effect(meter),
        ..AudioManagerSettings::default()
    };
    (settings, reader)
}

pub fn render_buffer(audio: &mut MockAudio) {
    render_manager_buffer(audio.manager_mut());
}

/// Renders one buffer of a manager that is not wrapped in an audio system.
pub fn render_manager_buffer(manager: &mut AudioManager<MockBackend>) {
    let backend = manager.backend_mut();
    backend.on_start_processing();
    backend.process();
}

/// Renders at least `duration` of audio.
pub fn render(audio: &mut MockAudio, sample_rate: u32, duration: Duration) {
    render_manager(audio.manager_mut(), sample_rate, duration);
}

/// Renders at least `duration` of audio of a manager that is not wrapped in an audio system.
pub fn render_manager(
    manager: &mut AudioManager<MockBackend>,
    sample_rate: u32,
    duration: Duration,
) {
    let frames = (duration.as_secs_f64() * f64::from(sample_rate)).ceil() as usize;
    for _ in 0..=frames.div_ceil(BUFFER_FRAMES) {
        render_manager_buffer(manager);
    }
}

/// Polls `condition` until it holds, panicking after a bounded wait. The decode thread runs
/// beside the mock render loop, so some conditions arrive after real time, not rendered time.
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(WAIT_STEP);
    }
}
