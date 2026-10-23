use std::path::PathBuf;

/// Everything that can go wrong with a playing video. None of it stops the game: the video is
/// dropped, the error is logged once and the menu stays black.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VideoError {
    #[error(
        "Windows Media Foundation is not available ({0}); Windows N editions need the Media \
         Feature Pack (Settings > Apps > Optional features)"
    )]
    MediaFoundationMissing(String),
    #[error("Media Foundation cannot {action}: {reason}")]
    MediaFoundation {
        action: &'static str,
        reason: String,
    },
    #[error(
        "the menu video {0} is missing; the installer's video lane converts it from the \
         game's videos"
    )]
    FileMissing(PathBuf),
    #[error("the video has no frames to loop")]
    NoFrames,
    #[error("unsupported video frame: {0}")]
    UnsupportedFrame(String),
    #[error("cannot start the video thread: {0}")]
    Thread(String),
}
