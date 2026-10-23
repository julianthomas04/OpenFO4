//! A looping player on its own thread: Media Foundation decodes the file, every frame is placed on
//! a continuous timeline ([`LoopTimeline`]) and handed over when that timeline reaches it. At the
//! end of the file the reader seeks back to the start and the timeline carries on, so the first
//! frame of the next round follows the last of the previous one without a gap.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::error::VideoError;
use super::frame::DecodedFrame;

const THREAD_NAME: &str = "video";

/// The longest a paced wait goes without checking for a stop request.
const STOP_POLL: Duration = Duration::from_millis(20);

/// The first fatal error of a player, set by its thread and read by its owner.
#[derive(Debug, Default)]
struct FailureSlot(Mutex<Option<VideoError>>);

impl FailureSlot {
    fn set(&self, error: VideoError) {
        let mut slot = self.lock();
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn take(&self) -> Option<VideoError> {
        self.lock().take()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<VideoError>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// What paces the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// Each frame is delivered when real time reaches its place on the timeline.
    Clock,
    /// Frames are delivered as fast as they decode; for measuring the stream, never for display.
    Unthrottled,
}

pub struct Player {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    failure: Arc<FailureSlot>,
}

impl Player {
    /// Starts playing `path` from the first frame, calling `on_frame` on the video thread for
    /// every frame as it falls due. Only setup errors return here; later ones come from
    /// [`Self::take_failure`].
    pub fn start(
        path: &Path,
        on_frame: impl Fn(DecodedFrame) + Send + 'static,
    ) -> Result<Self, VideoError> {
        Self::start_paced(path, Pacing::Clock, on_frame)
    }

    /// Like [`Self::start`] with the frames paced as `pacing` says.
    pub fn start_paced(
        path: &Path,
        pacing: Pacing,
        on_frame: impl Fn(DecodedFrame) + Send + 'static,
    ) -> Result<Self, VideoError> {
        Self::start_from(path, pacing, Duration::ZERO, on_frame)
    }

    /// Like [`Self::start_paced`] with the first round starting `from` into the file, so a test
    /// reaches the loop point without playing the whole video. Later rounds start at the beginning.
    pub fn start_from(
        path: &Path,
        pacing: Pacing,
        from: Duration,
        on_frame: impl Fn(DecodedFrame) + Send + 'static,
    ) -> Result<Self, VideoError> {
        if !path.is_file() {
            return Err(VideoError::FileMissing(path.to_path_buf()));
        }
        decoder::ensure_available()?;
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(FailureSlot::default());
        let playback = Playback {
            path: path.to_path_buf(),
            pacing,
            from,
            stop: stop.clone(),
        };
        let thread_failure = failure.clone();
        let thread = std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                if let Err(error) = decoder::play(&playback, &on_frame) {
                    thread_failure.set(error);
                }
            })
            .map_err(|error| VideoError::Thread(error.to_string()))?;
        Ok(Self {
            stop,
            thread: Some(thread),
            failure,
        })
    }

    /// The fatal error that stopped the video, once.
    pub fn take_failure(&self) -> Option<VideoError> {
        self.failure.take()
    }

    /// Stops the player and waits until its thread has finished.
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the video thread panicked");
        }
    }
}

impl Drop for Player {
    /// Dropping stops the video without waiting: the thread finishes on its own.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// What the video thread plays and how.
struct Playback {
    path: PathBuf,
    pacing: Pacing,
    from: Duration,
    stop: Arc<AtomicBool>,
}

impl Playback {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Waits until `due`; false when a stop request came first.
    fn wait_until(&self, due: Instant) -> bool {
        loop {
            if self.stopped() {
                return false;
            }
            let remaining = due.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return true;
            }
            spin_sleep::sleep(remaining.min(STOP_POLL));
        }
    }
}

#[cfg(windows)]
mod decoder {
    use std::time::{Duration, Instant};

    use super::super::error::VideoError;
    use super::super::frame::{DecodedFrame, FrameTiming};
    use super::super::media_foundation::{MediaFoundation, Session};
    use super::super::reader::VideoReader;
    use super::super::timeline::LoopTimeline;
    use super::{Pacing, Playback};

    pub fn ensure_available() -> Result<(), VideoError> {
        MediaFoundation::get().map(|_| ())
    }

    pub fn play(playback: &Playback, on_frame: &impl Fn(DecodedFrame)) -> Result<(), VideoError> {
        let session = Session::start(MediaFoundation::get()?)?;
        let mut reader = VideoReader::open(&session, &playback.path)?;
        if !playback.from.is_zero() {
            reader.seek(playback.from)?;
        }
        let mut timeline = LoopTimeline::default();
        let mut started: Option<Instant> = None;
        let mut frames_this_round = 0_u32;
        while !playback.stopped() {
            let Some(sample) = reader.read()? else {
                if frames_this_round == 0 {
                    return Err(VideoError::NoFrames);
                }
                reader.seek(Duration::ZERO)?;
                timeline.start_round();
                frames_this_round = 0;
                continue;
            };
            frames_this_round += 1;
            let running_time = timeline.place(sample.stream_time, sample.duration)?;
            if playback.pacing == Pacing::Clock {
                let start = *started.get_or_insert_with(Instant::now);
                if !playback.wait_until(start + running_time) {
                    break;
                }
            }
            let format = reader.format();
            on_frame(DecodedFrame::new(
                format.size,
                sample.data,
                format.conversion,
                format.siting,
                FrameTiming {
                    stream_time: sample.stream_time,
                    running_time,
                },
            )?);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod decoder {
    use super::super::error::VideoError;
    use super::super::frame::DecodedFrame;
    use super::Playback;

    const UNAVAILABLE: &str = "Media Foundation exists only on Windows";

    pub fn ensure_available() -> Result<(), VideoError> {
        Err(VideoError::MediaFoundationMissing(UNAVAILABLE.into()))
    }

    pub fn play(_: &Playback, _: &impl Fn(DecodedFrame)) -> Result<(), VideoError> {
        ensure_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_failure_is_kept_and_it_is_taken_once() {
        let slot = FailureSlot::default();
        slot.set(VideoError::NoFrames);
        slot.set(VideoError::Thread("later".into()));
        assert_eq!(slot.take(), Some(VideoError::NoFrames));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn a_missing_file_is_reported_before_anything_starts() {
        let error = Player::start(Path::new("definitely/not/here.mp4"), |_| {})
            .err()
            .expect("a missing file cannot start");
        assert_eq!(
            error,
            VideoError::FileMissing("definitely/not/here.mp4".into())
        );
    }

    #[test]
    fn a_stop_request_ends_a_paced_wait_early() {
        let playback = Playback {
            path: PathBuf::new(),
            pacing: Pacing::Clock,
            from: Duration::ZERO,
            stop: Arc::new(AtomicBool::new(true)),
        };
        let started = Instant::now();
        assert!(!playback.wait_until(started + Duration::from_secs(10)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_due_time_in_the_past_does_not_wait() {
        let playback = Playback {
            path: PathBuf::new(),
            pacing: Pacing::Clock,
            from: Duration::ZERO,
            stop: Arc::new(AtomicBool::new(false)),
        };
        assert!(playback.wait_until(Instant::now()));
    }
}
