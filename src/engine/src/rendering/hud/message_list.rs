//! The message stack without layout or drawing: how many lines show at once, how a line's clip
//! plays through the item timeline, and where lines wait when more arrive than fit. Every number
//! is a field of [`ListSpec`], read from the movie. The movie data gives a queue and removal from
//! the front of the shown lines; that the clip plays straight through and that lines wait for a
//! free slot is chosen here, not read (see `OBSERVED` in `messages.rs`).

use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ListSpec {
    /// Lines shown at once.
    pub max_shown: usize,
    /// Gap between two lines, in stage pixels.
    pub spacing: f32,
    pub fps: f32,
    /// The frame a line's clip starts at (label `FadeIn`).
    pub fade_in_frame: u32,
    /// The frame the fade-out part of the clip starts at (label `FadeOut`): the fade-in is over.
    pub fade_out_frame: u32,
    /// The clip's last frame; the line leaves after it.
    pub last_frame: u32,
}

/// A line on show and where its item clip is on its timeline.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ShownMessage {
    pub text: Arc<str>,
    pub frame: f32,
}

/// Lines in order of arrival, the oldest first (the movie removes from the front of its array).
/// At most `max_shown` lines wait; one more drops the oldest waiting line.
pub(super) struct MessageList {
    spec: ListSpec,
    shown: Vec<ShownMessage>,
    waiting: VecDeque<Arc<str>>,
}

impl MessageList {
    pub(super) fn new(spec: ListSpec) -> Self {
        Self {
            spec,
            shown: Vec::new(),
            waiting: VecDeque::new(),
        }
    }

    /// With a free slot the line starts at the fade-in frame; otherwise it waits for one. A text
    /// identical to a line on show or waiting is not added again and does not restart that line (measured in
    /// the original, see `OBSERVED` in `messages.rs`).
    pub(super) fn push(&mut self, text: Arc<str>) {
        if self.shown.iter().any(|message| message.text == text) || self.waiting.contains(&text) {
            return;
        }
        if self.shown.len() < self.spec.max_shown {
            self.start(text);
            return;
        }
        if self.waiting.len() >= self.spec.max_shown
            && let Some(dropped) = self.waiting.pop_front()
        {
            tracing::debug!("message list full: dropped the oldest waiting line {dropped:?}");
        }
        self.waiting.push_back(text);
    }

    /// Plays every shown clip on by `dt` seconds; a clip past its last frame leaves, and waiting
    /// lines take the freed slots in order.
    pub(super) fn advance(&mut self, dt: f32) {
        let step = (dt * self.spec.fps).max(0.0);
        let end = (self.spec.last_frame + 1) as f32;
        for message in &mut self.shown {
            message.frame += step;
        }
        self.shown.retain(|message| message.frame < end);
        while self.shown.len() < self.spec.max_shown {
            let Some(text) = self.waiting.pop_front() else {
                break;
            };
            self.start(text);
        }
    }

    pub(super) fn shown(&self) -> &[ShownMessage] {
        &self.shown
    }

    fn start(&mut self, text: Arc<str>) {
        self.shown.push(ShownMessage {
            text,
            frame: self.spec.fade_in_frame as f32,
        });
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "frames advance in exactly representable steps"
)]
mod tests {
    use super::*;

    const SPEC: ListSpec = ListSpec {
        max_shown: 3,
        spacing: 5.0,
        fps: 10.0,
        fade_in_frame: 3,
        fade_out_frame: 6,
        last_frame: 10,
    };

    fn list() -> MessageList {
        MessageList::new(SPEC)
    }

    fn texts(list: &MessageList) -> Vec<&str> {
        list.shown().iter().map(|m| &*m.text).collect()
    }

    fn frames(list: &MessageList) -> Vec<f32> {
        list.shown().iter().map(|m| m.frame).collect()
    }

    fn push(list: &mut MessageList, text: &str) {
        list.push(text.into());
    }

    #[test]
    fn a_text_identical_to_a_shown_line_is_not_added_and_does_not_extend_it() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.5);
        push(&mut list, "a");
        assert_eq!(texts(&list), ["a"]);
        assert_eq!(frames(&list), [8.0]);
        list.advance(0.3);
        assert_eq!(texts(&list), Vec::<&str>::new());
    }

    #[test]
    fn a_text_identical_to_a_waiting_line_is_not_queued_again() {
        let mut list = list();
        for text in ["a", "b", "c", "d"] {
            push(&mut list, text);
        }
        push(&mut list, "d");
        list.advance(1.0);
        assert_eq!(texts(&list), ["d"]);
    }

    #[test]
    fn a_different_text_while_one_is_shown_is_still_added() {
        let mut list = list();
        push(&mut list, "a");
        push(&mut list, "b");
        assert_eq!(texts(&list), ["a", "b"]);
    }

    #[test]
    fn the_same_text_may_come_again_once_its_line_has_left() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(2.0);
        assert_eq!(texts(&list), Vec::<&str>::new());
        push(&mut list, "a");
        assert_eq!(texts(&list), ["a"]);
    }

    #[test]
    fn a_line_with_a_free_slot_starts_at_the_fade_in_frame() {
        let mut list = list();
        push(&mut list, "a");
        assert_eq!(frames(&list), [3.0]);
    }

    #[test]
    fn lines_play_on_at_the_movie_frame_rate() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.5);
        assert_eq!(frames(&list), [8.0]);
    }

    #[test]
    fn a_line_stays_through_its_last_frame_and_leaves_after_it() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.79);
        assert_eq!(texts(&list), ["a"]);
        list.advance(0.02);
        assert_eq!(list.shown(), []);
    }

    #[test]
    fn time_never_runs_backwards() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(-1.0);
        list.advance(f32::NAN);
        assert_eq!(frames(&list), [3.0]);
    }

    #[test]
    fn lines_show_in_order_of_arrival_up_to_the_limit() {
        let mut list = list();
        for text in ["a", "b", "c", "d"] {
            push(&mut list, text);
        }
        assert_eq!(texts(&list), ["a", "b", "c"]);
    }

    #[test]
    fn a_line_over_the_limit_leaves_every_shown_line_where_it_is() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.5);
        push(&mut list, "b");
        push(&mut list, "c");
        let before = frames(&list);
        push(&mut list, "d");
        assert_eq!(frames(&list), before);
    }

    #[test]
    fn no_more_lines_wait_than_the_list_shows_and_the_oldest_waiting_one_is_dropped() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.3);
        for text in ["b", "c", "d", "e", "f", "g", "h", "i"] {
            push(&mut list, text);
        }
        list.advance(0.5);
        assert_eq!(texts(&list), ["b", "c", "g"]);
        list.advance(0.4);
        assert_eq!(texts(&list), ["g", "h", "i"]);
        list.advance(1.0);
        assert_eq!(list.shown(), []);
    }

    #[test]
    fn a_waiting_line_starts_when_a_slot_frees_and_not_before() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.3);
        push(&mut list, "b");
        push(&mut list, "c");
        push(&mut list, "d");
        list.advance(0.4);
        assert_eq!(texts(&list), ["a", "b", "c"]);
        list.advance(0.1);
        assert_eq!(texts(&list), ["b", "c", "d"]);
        assert_eq!(frames(&list)[2], 3.0);
    }

    #[test]
    fn one_freed_slot_takes_one_waiting_line_in_order() {
        let mut list = list();
        push(&mut list, "a");
        list.advance(0.3);
        for text in ["b", "c", "d", "e", "f"] {
            push(&mut list, text);
        }
        list.advance(0.5);
        assert_eq!(texts(&list), ["b", "c", "d"]);
    }

    #[test]
    fn an_empty_list_stays_empty() {
        let mut list = list();
        list.advance(1.0);
        assert_eq!(list.shown(), []);
    }
}
