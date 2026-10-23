use crate::game_flow::Phase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicCommand {
    StartMenu,
    StopMenu,
    StartWorld,
    StopWorld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Silent,
    Menu,
    World,
}

/// Decides which music source is wanted from the game flow's phase and emits commands only when
/// that changes: the menu music in the menu phases, the world music from `Phase::Revealing`
/// on, silence in between (`Loading`).
#[derive(Debug)]
pub struct MusicDirector {
    menu_enabled: bool,
    source: Source,
}

impl MusicDirector {
    pub fn new(menu_enabled: bool) -> Self {
        Self {
            menu_enabled,
            source: Source::Silent,
        }
    }

    fn wanted(&self, phase: Phase) -> Source {
        match phase {
            Phase::MainMenu | Phase::SpawnSelect if self.menu_enabled => Source::Menu,
            Phase::MainMenu | Phase::SpawnSelect | Phase::Loading => Source::Silent,
            Phase::Revealing | Phase::InGame => Source::World,
        }
    }

    /// The commands that bring the playing source to the wanted one: the old source stops before
    /// the new one starts.
    pub fn update(&mut self, phase: Phase) -> Vec<MusicCommand> {
        let wanted = self.wanted(phase);
        if wanted == self.source {
            return Vec::new();
        }
        let stop = match self.source {
            Source::Menu => Some(MusicCommand::StopMenu),
            Source::World => Some(MusicCommand::StopWorld),
            Source::Silent => None,
        };
        let start = match wanted {
            Source::Menu => Some(MusicCommand::StartMenu),
            Source::World => Some(MusicCommand::StartWorld),
            Source::Silent => None,
        };
        self.source = wanted;
        stop.into_iter().chain(start).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MusicCommand::{StartMenu, StartWorld, StopMenu, StopWorld};

    fn commands(menu_enabled: bool, phases: &[Phase]) -> Vec<Vec<MusicCommand>> {
        let mut director = MusicDirector::new(menu_enabled);
        phases.iter().map(|&phase| director.update(phase)).collect()
    }

    #[test]
    fn booting_into_the_menu_starts_the_menu_music_once() {
        assert_eq!(
            commands(true, &[Phase::MainMenu, Phase::MainMenu, Phase::MainMenu]),
            [vec![StartMenu], vec![], vec![]]
        );
    }

    #[test]
    fn moving_between_menu_phases_does_not_restart_the_music() {
        assert_eq!(
            commands(
                true,
                &[Phase::MainMenu, Phase::SpawnSelect, Phase::MainMenu]
            ),
            [vec![StartMenu], vec![], vec![]]
        );
    }

    #[test]
    fn the_menu_music_stops_when_loading_starts_and_the_world_music_starts_at_revealing() {
        assert_eq!(
            commands(
                true,
                &[
                    Phase::SpawnSelect,
                    Phase::Loading,
                    Phase::Loading,
                    Phase::Revealing,
                    Phase::InGame
                ]
            ),
            [
                vec![StartMenu],
                vec![StopMenu],
                vec![],
                vec![StartWorld],
                vec![]
            ]
        );
    }

    #[test]
    fn returning_to_the_menu_stops_the_world_music_before_the_menu_music_starts() {
        assert_eq!(
            commands(true, &[Phase::InGame, Phase::MainMenu, Phase::Loading]),
            [vec![StartWorld], vec![StopWorld, StartMenu], vec![StopMenu]]
        );
    }

    #[test]
    fn a_direct_start_plays_the_world_music_and_never_the_menu_music() {
        assert_eq!(
            commands(true, &[Phase::Loading, Phase::Revealing, Phase::InGame]),
            [vec![], vec![StartWorld], vec![]]
        );
    }

    #[test]
    fn disabled_menu_music_never_plays_but_the_world_music_does() {
        assert_eq!(
            commands(
                false,
                &[
                    Phase::MainMenu,
                    Phase::Loading,
                    Phase::Revealing,
                    Phase::MainMenu
                ]
            ),
            [vec![], vec![], vec![StartWorld], vec![StopWorld]]
        );
    }
}
