//! The main menu background video: a looping Media Foundation player that hands decoded frames to
//! the render thread, and the menu-phase lifecycle around it. Drawing the frames is the menu
//! renderer's job (`rendering/menu/video_layer.rs`).

mod colour;
mod error;
mod fit;
mod frame;
mod lifecycle;
mod mailbox;
#[cfg(windows)]
mod media_foundation;
mod menu_video;
mod player;
#[cfg(windows)]
mod reader;
mod settings;
mod timeline;

pub use colour::{ChromaSiting, ColourError, ColourMatrix, ColourRange, YuvToRgb};
pub use error::VideoError;
pub use fit::fit_rect;
pub use frame::{DecodedFrame, FrameTiming, PlanarFrame, Plane};
pub use lifecycle::{Lifecycle, VideoCommand};
pub use mailbox::Mailbox;
pub use menu_video::MenuVideo;
pub use player::{Pacing, Player};
pub use settings::{MAIN_MENU_VIDEO_SOURCE, MenuVideoSettings};
