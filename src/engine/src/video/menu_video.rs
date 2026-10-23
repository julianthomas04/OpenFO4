use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::error::VideoError;
use super::frame::DecodedFrame;
use super::lifecycle::{Lifecycle, VideoCommand};
use super::mailbox::Mailbox;
use super::player::Player;
use super::settings::{MAIN_MENU_VIDEO_SOURCE, MenuVideoSettings};
use crate::assets::layout;

/// Where a player delivers its frames, on the video thread.
type FrameSink = Arc<dyn Fn(DecodedFrame) + Send + Sync>;

/// Starts the player of one run; a test substitutes a source it drives itself.
type Starter = Box<dyn FnMut(&Path, FrameSink) -> Result<Option<Player>, VideoError> + Send>;

/// The main menu video: plays while a menu screen shows, from its first frame each time, and
/// leaves its latest decoded frame for the render thread.
pub struct MenuVideo {
    lifecycle: Lifecycle,
    path: PathBuf,
    /// Replaced at every start and stop: a player that is still winding down writes into the
    /// mailbox it was started with, never into the next run's.
    frames: Arc<Mailbox<DecodedFrame>>,
    player: Option<Player>,
    start: Starter,
    abandoned: bool,
}

impl MenuVideo {
    pub fn new(settings: MenuVideoSettings, asset_root: &Path) -> Self {
        Self::with_starter(
            settings,
            asset_root,
            Box::new(|path, sink| Player::start(path, move |frame| sink(frame)).map(Some)),
        )
    }

    fn with_starter(settings: MenuVideoSettings, asset_root: &Path, start: Starter) -> Self {
        Self {
            lifecycle: Lifecycle::new(settings.enabled),
            path: PathBuf::from(layout::video_path(
                &asset_root.to_string_lossy(),
                MAIN_MENU_VIDEO_SOURCE,
            )),
            frames: Arc::default(),
            player: None,
            start,
            abandoned: false,
        }
    }

    /// Gives the video up after `error`: logs it once, stops and frees the player and leaves the
    /// menu without a video for the rest of the run.
    pub fn abandon(&mut self, error: &VideoError) {
        tracing::error!("the main menu video stopped and stays off: {error}");
        self.player = None;
        self.frames = Arc::default();
        self.lifecycle.disable();
        self.abandoned = true;
    }

    /// Starts or stops the video as `menu_shown` asks, and fails when the player did. Returns the
    /// command it executed, so a stop can release what the frames were drawn into.
    pub fn update(&mut self, menu_shown: bool) -> Result<Option<VideoCommand>, VideoError> {
        if self.abandoned {
            return Ok(None);
        }
        let command = self.lifecycle.update(menu_shown);
        match command {
            Some(VideoCommand::Start) => {
                self.frames = Arc::default();
                let frames = self.frames.clone();
                self.player = (self.start)(
                    &self.path,
                    Arc::new(move |frame| {
                        frames.put(frame);
                    }),
                )?;
            }
            Some(VideoCommand::Stop) => {
                self.player = None;
                self.frames = Arc::default();
            }
            None => {}
        }
        if let Some(error) = self.player.as_ref().and_then(Player::take_failure) {
            return Err(error);
        }
        Ok(command)
    }

    /// The newest frame decoded since the last call.
    pub fn take_frame(&self) -> Option<DecodedFrame> {
        self.frames.take()
    }

    pub fn is_playing(&self) -> bool {
        self.lifecycle.is_playing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_video_is_read_from_the_converted_video_folder() {
        let video = MenuVideo::new(
            MenuVideoSettings { enabled: true },
            Path::new("root/assets"),
        );
        assert_eq!(
            video.path,
            PathBuf::from("root/assets/video/mainmenuloop.mp4")
        );
    }

    #[test]
    fn a_disabled_video_never_starts_or_asks_for_a_file() {
        let mut video = MenuVideo::new(MenuVideoSettings { enabled: false }, Path::new("missing"));
        assert_eq!(video.update(true).unwrap(), None);
        assert!(!video.is_playing());
        assert!(video.take_frame().is_none());
    }

    #[test]
    fn a_missing_file_fails_loudly_when_the_menu_opens() {
        let mut video = MenuVideo::new(MenuVideoSettings { enabled: true }, Path::new("missing"));
        let error = video.update(true).unwrap_err();
        assert!(matches!(error, VideoError::FileMissing(_)));
    }

    type Sinks = Arc<std::sync::Mutex<Vec<FrameSink>>>;

    /// A video whose frames the test delivers itself, from "players" that never run.
    fn driven_video() -> (MenuVideo, Sinks) {
        let sinks = Sinks::default();
        let recorded = sinks.clone();
        let video = MenuVideo::with_starter(
            MenuVideoSettings { enabled: true },
            Path::new("unused"),
            Box::new(move |_, sink| {
                recorded.lock().unwrap().push(sink);
                Ok(None)
            }),
        );
        (video, sinks)
    }

    fn deliver(sinks: &Sinks, run: usize) {
        let sink = sinks.lock().unwrap()[run].clone();
        sink(DecodedFrame::for_test());
    }

    #[test]
    fn a_frame_a_stopped_run_delivers_late_never_reaches_the_next_run() {
        let (mut video, sinks) = driven_video();
        video.update(true).unwrap();
        deliver(&sinks, 0);
        assert!(video.take_frame().is_some(), "a live run's frame arrives");

        video.update(false).unwrap();
        deliver(&sinks, 0);
        assert!(video.take_frame().is_none(), "late frame after the stop");

        video.update(true).unwrap();
        deliver(&sinks, 0);
        assert!(video.take_frame().is_none(), "late frame after the restart");
        deliver(&sinks, 1);
        assert!(video.take_frame().is_some(), "the new run's frame arrives");
    }

    #[test]
    fn an_abandoned_video_is_not_playing_any_more() {
        let (mut video, _sinks) = driven_video();
        video.update(true).unwrap();
        assert!(video.is_playing());
        video.abandon(&VideoError::NoFrames);
        assert!(!video.is_playing());
    }

    #[test]
    fn an_abandoned_video_stays_off_and_drops_its_frames() {
        let mut video = MenuVideo::new(MenuVideoSettings { enabled: true }, Path::new("missing"));
        video.abandon(&VideoError::NoFrames);
        assert!(video.take_frame().is_none());
        assert_eq!(video.update(true).unwrap(), None);
    }

    #[test]
    fn nothing_starts_while_no_menu_shows() {
        let mut video = MenuVideo::new(MenuVideoSettings { enabled: true }, Path::new("missing"));
        assert_eq!(video.update(false).unwrap(), None);
    }
}
