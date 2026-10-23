//! The animated Vault Boy art box shared by SPECIAL, PERKS and the quest log.

use egui::Pos2;

use crate::assets::pipboy_art::ArtClip;
use crate::rendering::pipboy::tab::TabContext;

/// Draws `clip` at `origin`, `started_at` being the `seconds_open` it began playing.
pub fn draw(cx: &mut TabContext<'_>, clip: &ArtClip, origin: Pos2, started_at: f64) {
    let frame = clip.frame_at((cx.seconds_open - started_at).max(0.0), None);
    let white = cx.shade(1.0);
    cx.draw_clip(clip, frame, origin, 1.0, white);
}
