//! The transient message line: one line of text above the hint bar that fades out after a few
//! seconds. Rejected actions and page notices share it.
//!
//! No interface movie defines this line, so every value below is a native-only stand-in, named
//! and labelled as one, until the user decides how the game's own refusal messages are shown.

use egui::{Pos2, Rect, Vec2};

use super::paint::{self, BLACK, TextField, TextStyle, faded, shade, text_field};
use super::tab::TabContext;

/// Stand-in: seconds a message stays fully opaque.
const VISIBLE_SECONDS: f64 = 3.0;
/// Stand-in: seconds a message takes to fade out.
const FADE_SECONDS: f64 = 0.5;
/// Stand-in: the message's font size.
const TEXT_SIZE: f32 = 20.0;
/// Stand-in: the width of the message's text field.
const FIELD_WIDTH: f32 = 700.0;
/// Stand-in: the height of the black backdrop behind the message.
const BACKDROP_HEIGHT: f32 = 26.0;
/// Stand-in: the opacity of the black backdrop behind the message.
const BACKDROP_ALPHA: f32 = 0.7;

/// Stand-in: the top of the line on the shell stage, in stage pixels; the line is centred on the
/// stage.
pub const SHELL_LINE_TOP: f32 = 545.0;
/// Stand-in: the top of the line on the perk chart stage, in stage pixels; the line is centred on
/// the stage.
pub const PERK_CHART_LINE_TOP: f32 = 636.0;

/// Opacity of a message `age` seconds after it appeared: full, then fading to nothing.
pub fn opacity(age: f64) -> f32 {
    if age < 0.0 {
        return 0.0;
    }
    ((VISIBLE_SECONDS - age) / FADE_SECONDS).clamp(0.0, 1.0) as f32
}

#[derive(Debug)]
struct Message {
    text: String,
    shown_at: f64,
}

/// The current message, timed on the screen's open clock.
#[derive(Debug, Default)]
pub struct NoticeLine {
    message: Option<Message>,
}

impl NoticeLine {
    pub fn show(&mut self, text: String, seconds: f64) {
        self.message = Some(Message {
            text,
            shown_at: seconds,
        });
    }

    pub fn clear(&mut self) {
        self.message = None;
    }

    pub fn text(&self) -> Option<&str> {
        self.message.as_ref().map(|message| message.text.as_str())
    }

    /// Draws the message centred on `centre_x` with its top at `top`; drops it once faded.
    pub fn draw(&mut self, cx: &TabContext<'_>, centre_x: f32, top: f32) {
        let anchor = Pos2::new(centre_x, top);
        let Some(message) = &self.message else {
            return;
        };
        let alpha = opacity(cx.seconds_open - message.shown_at);
        if alpha <= 0.0 {
            self.message = None;
            return;
        }
        let backdrop = Rect::from_center_size(
            anchor + Vec2::new(0.0, BACKDROP_HEIGHT / 2.0),
            Vec2::new(FIELD_WIDTH, BACKDROP_HEIGHT),
        );
        paint::fill(cx.painter, backdrop, faded(BLACK, BACKDROP_ALPHA * alpha));
        let field = TextField::centre(
            Pos2::new(anchor.x - FIELD_WIDTH / 2.0, anchor.y + 2.0),
            FIELD_WIDTH,
        );
        let colour = faded(shade(cx.tint, 1.0), alpha);
        text_field(
            cx.painter,
            field,
            &message.text,
            TextStyle::bold(TEXT_SIZE, colour),
        );
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_opaque_then_fades_and_is_gone_after_three_seconds() {
        assert!((opacity(0.0) - 1.0).abs() < 1e-6);
        assert!((opacity(2.5) - 1.0).abs() < 1e-6);
        assert!((opacity(2.75) - 0.5).abs() < 1e-6);
        assert_eq!(opacity(3.0), 0.0);
        assert_eq!(opacity(10.0), 0.0);
    }

    #[test]
    fn a_message_from_a_later_clock_than_now_is_invisible() {
        assert_eq!(opacity(-1.0), 0.0);
    }

    #[test]
    fn a_new_message_replaces_the_old_one_and_clear_removes_it() {
        let mut line = NoticeLine::default();
        line.show("first".into(), 0.0);
        line.show("second".into(), 1.0);
        assert_eq!(line.text(), Some("second"));
        line.clear();
        assert_eq!(line.text(), None);
    }
}
