use super::clip::{Clip, Cycle};

const MAX_DELTA_SECONDS: f32 = 0.1;
/// Looping clips shorter than this are treated as static poses.
const MIN_LOOP_DURATION_SECONDS: f32 = 1e-4;
/// Bounds the marker work of one advance however short the loop is.
const MAX_WRAPS_PER_ADVANCE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FiredMarker {
    /// Index into `Clip::markers`.
    pub marker_index: usize,
    pub time: f32,
}

#[derive(Clone, Debug)]
pub struct AnimPlayer {
    clip: usize,
    time: f32,
    speed: f32,
    state: PlayState,
    /// The marker at the exact starting time has not been considered yet.
    fresh: bool,
}

impl AnimPlayer {
    pub fn new(clip: usize, time: f32, speed: f32) -> Self {
        Self {
            clip,
            time,
            speed,
            state: PlayState::Playing,
            fresh: true,
        }
    }

    pub fn clip(&self) -> usize {
        self.clip
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn state(&self) -> PlayState {
        self.state
    }

    pub fn is_finished(&self) -> bool {
        self.state == PlayState::Finished
    }

    /// Jumps to `time` and resumes; the marker at `time` fires on the next advance.
    pub fn seek(&mut self, time: f32) {
        self.time = time;
        self.state = PlayState::Playing;
        self.fresh = true;
    }

    /// Changes direction or rate and resumes a finished player, e.g. to close a door by reversing.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        self.state = PlayState::Playing;
    }

    pub fn advance(&mut self, dt: f32, clip: &Clip, fired: &mut Vec<FiredMarker>) {
        if self.is_finished() || !dt.is_finite() || !self.speed.is_finite() {
            return;
        }
        let delta = dt.clamp(0.0, MAX_DELTA_SECONDS) * self.speed;
        if delta == 0.0 && !self.fresh {
            return;
        }
        let duration = clip.duration();
        match clip.cycle() {
            Cycle::Clamp => self.advance_clamped(delta, duration, clip, fired),
            Cycle::Loop if duration >= MIN_LOOP_DURATION_SECONDS => {
                self.advance_looped(delta, duration, clip, fired);
            }
            Cycle::Loop => self.time = 0.0,
        }
        self.fresh = false;
    }

    fn advance_clamped(
        &mut self,
        delta: f32,
        duration: f32,
        clip: &Clip,
        fired: &mut Vec<FiredMarker>,
    ) {
        let start = self.time.clamp(0.0, duration);
        let target = start + delta;
        let end = target.clamp(0.0, duration);
        fire(clip, start, end, self.fresh, delta >= 0.0, fired);
        self.time = end;
        let pose_clip = duration <= 0.0;
        if pose_clip || (delta > 0.0 && target >= duration) || (delta < 0.0 && target <= 0.0) {
            self.state = PlayState::Finished;
        }
    }

    fn advance_looped(
        &mut self,
        delta: f32,
        duration: f32,
        clip: &Clip,
        fired: &mut Vec<FiredMarker>,
    ) {
        let forward = delta >= 0.0;
        let max_delta = duration * MAX_WRAPS_PER_ADVANCE;
        let delta = delta.clamp(-max_delta, max_delta);
        let mut from = self.time.rem_euclid(duration);
        let mut include_from = self.fresh;
        let mut target = from + delta;
        if forward {
            while target >= duration {
                fire(clip, from, duration, include_from, true, fired);
                from = 0.0;
                include_from = true;
                target -= duration;
            }
        } else {
            while target < 0.0 {
                fire(clip, from, 0.0, include_from, false, fired);
                from = duration;
                include_from = true;
                target += duration;
            }
        }
        fire(clip, from, target, include_from, forward, fired);
        self.time = target;
    }
}

/// Fires markers travelling from `from` to `to`; `to` is always included, `from` only if asked.
fn fire(
    clip: &Clip,
    from: f32,
    to: f32,
    include_from: bool,
    forward: bool,
    fired: &mut Vec<FiredMarker>,
) {
    let (low, high, include_low, include_high) = if forward {
        (from, to, include_from, true)
    } else {
        (to, from, true, include_from)
    };
    let markers = clip.markers();
    let first = markers.partition_point(|m| {
        if include_low {
            m.time < low
        } else {
            m.time <= low
        }
    });
    let past = markers.partition_point(|m| {
        if include_high {
            m.time <= high
        } else {
            m.time < high
        }
    });
    if first >= past {
        return;
    }
    let as_fired = |index: usize| FiredMarker {
        marker_index: index,
        time: markers[index].time,
    };
    if forward {
        fired.extend((first..past).map(as_fired));
    } else {
        fired.extend((first..past).rev().map(as_fired));
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;
    use crate::anim::clip::Marker;

    /// Exactly representable in f32, so accumulated playback time has no rounding error.
    const STEP: f32 = 0.0625;

    fn clip(duration: f32, cycle: Cycle, marker_times: &[f32]) -> Clip {
        let markers = marker_times
            .iter()
            .map(|&time| Marker {
                time,
                text: format!("m{time}"),
            })
            .collect();
        Clip::new("c".to_owned(), duration, cycle, vec![], markers).unwrap()
    }

    fn advance(player: &mut AnimPlayer, dt: f32, clip: &Clip) -> Vec<usize> {
        let mut fired = Vec::new();
        player.advance(dt, clip, &mut fired);
        fired.iter().map(|f| f.marker_index).collect()
    }

    #[test]
    fn clamped_clip_finishes_exactly_at_its_duration() {
        let clip = clip(0.25, Cycle::Clamp, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        for _ in 0..3 {
            advance(&mut player, STEP, &clip);
            assert!(!player.is_finished());
        }
        advance(&mut player, STEP, &clip);
        assert_eq!(player.time(), 0.25);
        assert_eq!(player.state(), PlayState::Finished);
    }

    #[test]
    fn finished_player_ignores_further_advances() {
        let clip = clip(0.0625, Cycle::Clamp, &[0.0625]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        assert_eq!(advance(&mut player, STEP, &clip), [0]);
        assert_eq!(advance(&mut player, STEP, &clip), Vec::<usize>::new());
        assert_eq!(player.time(), 0.0625);
    }

    #[test]
    fn zero_duration_pose_clip_finishes_on_the_first_advance() {
        let clip = clip(0.0, Cycle::Clamp, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        advance(&mut player, STEP, &clip);
        assert!(player.is_finished());
        assert_eq!(player.time(), 0.0);
    }

    #[test]
    fn frame_delta_is_capped_at_a_tenth_of_a_second() {
        let clip = clip(10.0, Cycle::Clamp, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        advance(&mut player, 5.0, &clip);
        assert_eq!(player.time(), MAX_DELTA_SECONDS);
    }

    #[test]
    fn speed_scales_the_capped_delta() {
        let clip = clip(10.0, Cycle::Clamp, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 2.0);
        advance(&mut player, 5.0, &clip);
        assert_eq!(player.time(), 2.0 * MAX_DELTA_SECONDS);
    }

    #[test]
    fn non_finite_dt_or_speed_leaves_the_player_untouched() {
        let clip = clip(10.0, Cycle::Clamp, &[0.0]);
        let mut player = AnimPlayer::new(0, 1.0, 1.0);
        assert_eq!(advance(&mut player, f32::NAN, &clip), Vec::<usize>::new());
        let mut broken = AnimPlayer::new(0, 1.0, f32::INFINITY);
        assert_eq!(advance(&mut broken, STEP, &clip), Vec::<usize>::new());
        assert_eq!(player.time(), 1.0);
        assert_eq!(broken.time(), 1.0);
    }

    #[test]
    fn marker_at_the_starting_time_fires_once() {
        let clip = clip(1.0, Cycle::Clamp, &[0.0]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        assert_eq!(advance(&mut player, STEP, &clip), [0]);
        assert_eq!(advance(&mut player, STEP, &clip), Vec::<usize>::new());
    }

    #[test]
    fn marker_fires_on_the_step_that_lands_exactly_on_it_and_not_again() {
        let clip = clip(2.0, Cycle::Clamp, &[0.25]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        let mut fired_per_step = Vec::new();
        for _ in 0..6 {
            fired_per_step.push(advance(&mut player, STEP, &clip).len());
        }
        assert_eq!(fired_per_step, [0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn seek_makes_the_marker_at_the_new_time_fire_again() {
        let clip = clip(2.0, Cycle::Clamp, &[0.5]);
        let mut player = AnimPlayer::new(0, 0.5, 1.0);
        assert_eq!(advance(&mut player, STEP, &clip), [0]);
        player.seek(0.5);
        assert_eq!(advance(&mut player, STEP, &clip), [0]);
    }

    #[test]
    fn markers_crossed_in_one_step_fire_in_time_order() {
        let clip = clip(2.0, Cycle::Clamp, &[0.01, 0.02, 0.09]);
        let mut player = AnimPlayer::new(0, 0.005, 1.0);
        assert_eq!(advance(&mut player, MAX_DELTA_SECONDS, &clip), [0, 1, 2]);
    }

    #[test]
    fn reversed_playback_fires_markers_in_reverse_order_and_finishes_at_zero() {
        let clip = clip(1.0, Cycle::Clamp, &[0.25, 0.5, 0.75]);
        let mut player = AnimPlayer::new(0, 0.8, -10.0);
        // 0.8 - 0.625 = 0.175: crosses 0.75, 0.5 and 0.25 backwards.
        assert_eq!(advance(&mut player, STEP, &clip), [2, 1, 0]);
        assert!(!player.is_finished());
        advance(&mut player, STEP, &clip);
        assert_eq!(player.time(), 0.0);
        assert!(player.is_finished());
    }

    #[test]
    fn set_speed_resumes_a_finished_player_in_the_other_direction() {
        let clip = clip(0.0625, Cycle::Clamp, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        advance(&mut player, STEP, &clip);
        assert!(player.is_finished());
        player.set_speed(-1.0);
        assert!(!player.is_finished());
        advance(&mut player, STEP, &clip);
        assert_eq!(player.time(), 0.0);
        assert!(player.is_finished());
    }

    #[test]
    fn looping_clip_wraps_and_fires_markers_on_both_sides_of_the_seam() {
        let clip = clip(1.0, Cycle::Loop, &[0.25, 0.875]);
        let mut player = AnimPlayer::new(0, 0.75, 10.0);
        // 0.75 + 0.625 = 1.375, wrapping to 0.375.
        let fired = advance(&mut player, STEP, &clip);
        assert_eq!(fired, [1, 0]);
        assert_eq!(player.time(), 0.375);
        assert!(!player.is_finished());
    }

    #[test]
    fn looping_clip_never_finishes() {
        let clip = clip(0.5, Cycle::Loop, &[]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        for _ in 0..100 {
            advance(&mut player, STEP, &clip);
        }
        assert!(!player.is_finished());
        assert!((0.0..0.5).contains(&player.time()));
    }

    #[test]
    fn reversed_loop_wraps_below_zero_to_the_end() {
        let clip = clip(1.0, Cycle::Loop, &[]);
        let mut player = AnimPlayer::new(0, 0.25, -10.0);
        advance(&mut player, STEP, &clip);
        assert_eq!(player.time(), 0.625);
    }

    #[test]
    fn loop_shorter_than_the_minimum_is_a_static_pose() {
        let clip = clip(MIN_LOOP_DURATION_SECONDS / 2.0, Cycle::Loop, &[]);
        let mut player = AnimPlayer::new(0, 3.0, 1.0);
        advance(&mut player, STEP, &clip);
        assert_eq!(player.time(), 0.0);
        assert!(!player.is_finished());
    }

    #[test]
    fn wraps_per_advance_are_bounded_so_marker_work_stays_bounded() {
        let clip = clip(0.002, Cycle::Loop, &[0.001]);
        let mut player = AnimPlayer::new(0, 0.0, 1.0);
        let fired = advance(&mut player, MAX_DELTA_SECONDS, &clip);
        assert!(
            fired.len() <= MAX_WRAPS_PER_ADVANCE as usize + 1,
            "{} markers fired",
            fired.len()
        );
    }
}
