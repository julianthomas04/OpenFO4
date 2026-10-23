use super::clip::{Clip, Cycle};

const OPEN_MOTIONS: &[&str] = &["Open", "Opening", "OpenClose", "On"];
const CLOSE_MOTIONS: &[&str] = &["Close", "Closing", "Off"];
const CLOSED_POSE: &str = "Closed";
const OPENED_POSE: &str = "Opened";

/// Which clips move an object between its two states, resolved once per rig.
/// Pose clips are zero-duration clips holding a single state.
#[derive(Debug, PartialEq, Eq)]
pub struct ClipRoles {
    pub open: Option<usize>,
    /// Without a close motion the open motion plays in reverse.
    pub close: Option<usize>,
    pub closed_pose: Option<usize>,
    pub opened_pose: Option<usize>,
    /// The open clip was picked for lack of any conventionally named one.
    pub open_is_fallback: bool,
    /// The clip a non-activatable object plays forever: the first looping motion.
    pub auto_loop: Option<usize>,
}

impl ClipRoles {
    pub fn resolve(clips: &[Clip]) -> Self {
        let index_of = |name: &str| {
            clips
                .iter()
                .position(|clip| clip.name().eq_ignore_ascii_case(name))
        };
        let find = |names: &[&str]| names.iter().find_map(|name| index_of(name));
        let is_pose = |clip: usize| clips[clip].duration() <= 0.0;
        let closed_pose = index_of(CLOSED_POSE).filter(|&clip| is_pose(clip));
        let opened_pose = index_of(OPENED_POSE).filter(|&clip| is_pose(clip));
        let mut open = find(OPEN_MOTIONS);
        let mut open_is_fallback = false;
        if open.is_none() && opened_pose.is_none() {
            open = (0..clips.len())
                .find(|&clip| clips[clip].cycle() == Cycle::Clamp && !is_pose(clip));
            open_is_fallback = open.is_some();
        }
        let auto_loop =
            (0..clips.len()).find(|&clip| clips[clip].cycle() == Cycle::Loop && !is_pose(clip));
        Self {
            auto_loop,
            open,
            close: find(CLOSE_MOTIONS),
            closed_pose,
            opened_pose,
            open_is_fallback,
        }
    }

    pub fn describe(&self, clips: &[Clip]) -> String {
        let name = |clip: Option<usize>, otherwise: &str| {
            clip.map_or_else(
                || otherwise.to_owned(),
                |clip| clips[clip].name().to_owned(),
            )
        };
        let close_otherwise = if self.open.is_some() {
            "reverse"
        } else {
            "none"
        };
        format!(
            "open:{},close:{},closed:{}",
            name(self.open, "none"),
            name(self.close, close_otherwise),
            name(self.closed_pose, "rest")
        )
    }

    pub fn missing_open_motion_message(model_path: &str, clips: &[Clip], open: usize) -> String {
        format!(
            "anim: {model_path} has none of the clips {OPEN_MOTIONS:?}; playing {:?} instead",
            clips[open].name()
        )
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn clip(name: &str, duration: f32, cycle: Cycle) -> Clip {
        Clip::new(name.to_owned(), duration, cycle, vec![], vec![]).unwrap()
    }

    #[test]
    fn conventional_names_resolve_case_insensitively() {
        let clips = [
            clip("CLOSE", 1.0, Cycle::Clamp),
            clip("open", 1.0, Cycle::Clamp),
        ];
        let roles = ClipRoles::resolve(&clips);
        assert_eq!(roles.open, Some(1));
        assert_eq!(roles.close, Some(0));
        assert!(!roles.open_is_fallback);
    }

    #[test]
    fn open_names_are_tried_in_priority_order() {
        let clips = [
            clip("On", 1.0, Cycle::Clamp),
            clip("Opening", 1.0, Cycle::Clamp),
        ];
        assert_eq!(ClipRoles::resolve(&clips).open, Some(1));
    }

    #[test]
    fn pose_clips_must_have_zero_duration() {
        let clips = [
            clip("Closed", 0.0, Cycle::Clamp),
            clip("Opened", 2.0, Cycle::Clamp),
        ];
        let roles = ClipRoles::resolve(&clips);
        assert_eq!(roles.closed_pose, Some(0));
        assert_eq!(roles.opened_pose, None, "a moving clip is not a pose");
    }

    #[test]
    fn first_clamped_motion_is_the_fallback_open_clip() {
        let clips = [
            clip("Idle", 1.0, Cycle::Loop),
            clip("Pose", 0.0, Cycle::Clamp),
            clip("Swing", 1.0, Cycle::Clamp),
        ];
        let roles = ClipRoles::resolve(&clips);
        assert_eq!(roles.open, Some(2));
        assert!(roles.open_is_fallback);
    }

    #[test]
    fn opened_pose_suppresses_the_fallback() {
        let clips = [
            clip("Opened", 0.0, Cycle::Clamp),
            clip("Swing", 1.0, Cycle::Clamp),
        ];
        let roles = ClipRoles::resolve(&clips);
        assert_eq!(roles.open, None);
        assert!(!roles.open_is_fallback);
    }

    #[test]
    fn auto_loop_is_the_first_looping_motion() {
        let clips = [
            clip("Swing", 1.0, Cycle::Clamp),
            clip("Still", 0.0, Cycle::Loop),
            clip("Spin", 2.0, Cycle::Loop),
            clip("Spin2", 2.0, Cycle::Loop),
        ];
        assert_eq!(ClipRoles::resolve(&clips).auto_loop, Some(2));
        assert_eq!(ClipRoles::resolve(&clips[..1]).auto_loop, None);
    }

    #[test]
    fn no_clips_resolve_to_nothing() {
        let roles = ClipRoles::resolve(&[]);
        assert_eq!(roles.open, None);
        assert_eq!(roles.close, None);
        assert!(!roles.open_is_fallback);
    }

    #[test]
    fn describe_says_reverse_when_only_the_open_clip_exists() {
        let clips = [clip("Open", 1.0, Cycle::Clamp)];
        let roles = ClipRoles::resolve(&clips);
        assert_eq!(
            roles.describe(&clips),
            "open:Open,close:reverse,closed:rest"
        );
    }
}
