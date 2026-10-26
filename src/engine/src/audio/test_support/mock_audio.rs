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
/// The pause before a window of paced rendering, for the decoder threads of streaming sounds to
/// have refilled their buffers.
const DECODER_HEAD_START: Duration = Duration::from_millis(10);
/// The longest stretch rendered between two pauses. kira buffers about two seconds of a stream, so
/// a stream that was full at the start of a window is far from drained at its end.
const PACED_WINDOW: Duration = Duration::from_millis(250);

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

/// `render` for a system whose sounds stream, see `render_manager_paced`.
pub fn render_paced(audio: &mut MockAudio, sample_rate: u32, duration: Duration) {
    render_manager_paced(audio.manager_mut(), sample_rate, duration);
}

/// Renders at least `duration` of a manager whose sounds stream. The render runs far faster than
/// real time, so it can drain the buffer of a stream faster than the decoder thread beside it
/// refills it, and the stream then falls silent for a while. Pacing the render in windows, each
/// after a pause for the decoder, keeps the streams full: what a fade or a gain does over the
/// rendered time can then be read from the output.
pub fn render_manager_paced(
    manager: &mut AudioManager<MockBackend>,
    sample_rate: u32,
    duration: Duration,
) {
    let mut left = duration;
    while !left.is_zero() {
        let window = left.min(PACED_WINDOW);
        std::thread::sleep(DECODER_HEAD_START);
        render_manager(manager, sample_rate, window);
        left -= window;
    }
}

/// Renders `window` at a time, as `render_manager_paced` does, until a window has anything to hear,
/// and returns the loudest sample of that window; `0.0` when the wait limit passes first. The
/// sounds of a streaming system begin beside the rendering, so how soon one comes through depends
/// on how soon its decoder thread gets to run.
pub fn peak_when_heard(
    manager: &mut AudioManager<MockBackend>,
    peak: &PeakReader,
    sample_rate: u32,
    window: Duration,
) -> f32 {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        peak.take();
        render_manager_paced(manager, sample_rate, window);
        let heard = peak.take();
        if heard > 0.0 || Instant::now() >= deadline {
            return heard;
        }
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
