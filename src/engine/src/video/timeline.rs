use std::time::Duration;

use super::error::VideoError;

/// Places the frames of a looping video on one continuous playback timeline: within a round the
/// frames keep their spacing from the file, and the first frame of every new round follows the
/// end of the previous round's last frame, so the loop has no gap and no overlap.
#[derive(Debug, Default)]
pub struct LoopTimeline {
    /// Where the next frame starts when it begins a round.
    next: Duration,
    /// The running and stream time of the current round's first frame.
    round_origin: Option<(Duration, Duration)>,
}

impl LoopTimeline {
    /// The running time of the frame at `stream_time` that lasts `duration`.
    pub fn place(
        &mut self,
        stream_time: Duration,
        duration: Duration,
    ) -> Result<Duration, VideoError> {
        let (origin_running, origin_stream) =
            *self.round_origin.get_or_insert((self.next, stream_time));
        let since_origin = stream_time.checked_sub(origin_stream).ok_or_else(|| {
            VideoError::UnsupportedFrame(format!(
                "the frame at {stream_time:?} comes before the round's first frame at \
                 {origin_stream:?}"
            ))
        })?;
        let running = origin_running + since_origin;
        self.next = running + duration;
        Ok(running)
    }

    /// The next frame starts a new round.
    pub fn start_round(&mut self) {
        self.round_origin = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(33);

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn the_first_frame_starts_the_timeline_whatever_its_file_position() {
        let mut timeline = LoopTimeline::default();
        assert_eq!(timeline.place(ms(66), FRAME).unwrap(), Duration::ZERO);
        assert_eq!(timeline.place(ms(99), FRAME).unwrap(), ms(33));
    }

    #[test]
    fn a_new_round_follows_the_last_frame_of_the_previous_one() {
        let mut timeline = LoopTimeline::default();
        timeline.place(ms(0), FRAME).unwrap();
        assert_eq!(timeline.place(ms(33), FRAME).unwrap(), ms(33));
        timeline.start_round();
        assert_eq!(timeline.place(ms(66), FRAME).unwrap(), ms(66));
        assert_eq!(timeline.place(ms(99), FRAME).unwrap(), ms(99));
        timeline.start_round();
        assert_eq!(timeline.place(ms(66), FRAME).unwrap(), ms(132));
    }

    #[test]
    fn a_frame_before_the_round_start_is_refused() {
        let mut timeline = LoopTimeline::default();
        timeline.place(ms(100), FRAME).unwrap();
        assert!(timeline.place(ms(50), FRAME).is_err());
    }
}
