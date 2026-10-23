/// What the menu video should do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCommand {
    Start,
    Stop,
}

/// Decides when the menu video starts and stops from whether a menu screen is showing. Every
/// start plays from the first frame; moving between menu screens does not restart it.
#[derive(Debug, Clone, Copy)]
pub struct Lifecycle {
    enabled: bool,
    playing: bool,
}

impl Lifecycle {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            playing: false,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Turns the video off for good and reports it as not playing.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.playing = false;
    }

    pub fn update(&mut self, menu_shown: bool) -> Option<VideoCommand> {
        if !self.enabled || menu_shown == self.playing {
            return None;
        }
        self.playing = menu_shown;
        Some(if menu_shown {
            VideoCommand::Start
        } else {
            VideoCommand::Stop
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(enabled: bool, menu_shown: &[bool]) -> Vec<Option<VideoCommand>> {
        let mut lifecycle = Lifecycle::new(enabled);
        menu_shown
            .iter()
            .map(|&shown| lifecycle.update(shown))
            .collect()
    }

    #[test]
    fn a_disabled_lifecycle_is_not_playing_and_never_starts_again() {
        let mut lifecycle = Lifecycle::new(true);
        assert_eq!(lifecycle.update(true), Some(VideoCommand::Start));
        lifecycle.disable();
        assert!(!lifecycle.is_playing());
        assert_eq!(lifecycle.update(true), None);
        assert_eq!(lifecycle.update(false), None);
    }

    #[test]
    fn entering_the_menu_starts_the_video_once() {
        assert_eq!(
            commands(true, &[true, true, true]),
            [Some(VideoCommand::Start), None, None]
        );
    }

    #[test]
    fn leaving_the_menu_stops_the_video_once() {
        assert_eq!(
            commands(true, &[true, false, false, false]),
            [
                Some(VideoCommand::Start),
                Some(VideoCommand::Stop),
                None,
                None
            ]
        );
    }

    #[test]
    fn returning_to_the_menu_starts_it_again() {
        assert_eq!(
            commands(true, &[true, false, true]),
            [
                Some(VideoCommand::Start),
                Some(VideoCommand::Stop),
                Some(VideoCommand::Start)
            ]
        );
    }

    #[test]
    fn a_game_that_never_shows_the_menu_never_starts_it() {
        assert_eq!(commands(true, &[false, false]), [None, None]);
    }

    #[test]
    fn a_disabled_video_never_starts() {
        assert_eq!(commands(false, &[true, false, true]), [None, None, None]);
    }

    #[test]
    fn the_playing_flag_follows_the_commands() {
        let mut lifecycle = Lifecycle::new(true);
        assert!(!lifecycle.is_playing());
        lifecycle.update(true);
        assert!(lifecycle.is_playing());
        lifecycle.update(false);
        assert!(!lifecycle.is_playing());
    }
}
