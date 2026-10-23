//! Rank stars and the SPECIAL counters' stars, drawn from the chart's own star clips at the size
//! the chart's star shapes have.

use egui::{Color32, Pos2, Vec2};

use super::layout::ChartLayout;
use crate::rendering::pipboy::pages::perk_chart_layout::RankStar;
use crate::rendering::pipboy::paint::faded;
use crate::rendering::pipboy::tab::TabContext;

/// The clip of the three rank star states.
pub const RANK_STAR_CLIP: &str = "perkchart/rank_star";
/// The clip of a SPECIAL counter's star.
pub const SPECIAL_STAR_CLIP: &str = "perkchart/special_star";

/// Draws frame `frame` of the star clip `id` filling the box of `size` whose top-left corner is
/// `top_left`.
fn draw_star_clip(
    cx: &mut TabContext<'_>,
    id: &str,
    frame: usize,
    top_left: Pos2,
    size: Vec2,
    colour: Color32,
) {
    let Some(clip) = cx.art_library.clip(id) else {
        crate::utils::warn_once(&format!("pipboy-perk-chart-clip:{id}"), || {
            format!("the art manifest has no clip '{id}'")
        });
        return;
    };
    let [left, top, right, _] = clip.bounds;
    let scale = size.x / (right - left);
    let at = top_left - Vec2::new(left, top) * scale;
    cx.draw_clip(clip, frame, at, scale, colour);
}

/// The frame of the blinking star at `seconds`.
pub fn blink_frame(frames: usize, fps: f32, seconds: f64) -> usize {
    if frames == 0 {
        return 0;
    }
    (seconds * f64::from(fps)).floor().max(0.0) as usize % frames
}

pub fn draw_rank_star(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    top_left: Pos2,
    size: Vec2,
    star: RankStar,
) {
    let frames = layout.star_frames;
    let seconds = cx.seconds_open;
    let mut draw = |frame: usize, colour: Color32| {
        draw_star_clip(cx, RANK_STAR_CLIP, frame, top_left, size, colour);
    };
    match star {
        RankStar::Empty => draw(frames.empty, Color32::WHITE),
        RankStar::Full => draw(frames.full, Color32::WHITE),
        RankStar::Available => {
            draw(frames.empty, Color32::WHITE);
            let blink = &layout.star_blink;
            let frame = blink_frame(blink.fade.len(), blink.fps, seconds);
            let alpha = blink.fade.get(frame).copied().unwrap_or(0.0);
            draw(frames.full, faded(Color32::WHITE, alpha));
        }
    }
}

/// A star of a SPECIAL counter.
pub fn draw_special_star(cx: &mut TabContext<'_>, top_left: Pos2, size: Vec2) {
    draw_star_clip(cx, SPECIAL_STAR_CLIP, 0, top_left, size, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blink_runs_through_its_frames_and_repeats() {
        assert_eq!(blink_frame(40, 30.0, 0.0), 0);
        assert_eq!(blink_frame(40, 30.0, 1.0), 30);
        assert_eq!(blink_frame(40, 30.0, 40.0 / 30.0), 0);
    }

    #[test]
    fn a_clip_without_frames_stays_on_frame_zero() {
        assert_eq!(blink_frame(0, 30.0, 5.0), 0);
    }
}
