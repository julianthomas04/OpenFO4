//! Whether the game opens at the main menu or goes straight into the world.

use crate::rendering::pipboy_capture::CAPTURE_DIR_ENV;
use crate::rendering::shot::shot_requested;
use crate::spawn_cell::{SPAWN_CELL_VAR, SPAWN_VAR};

const SKIP_MENU_VAR: &str = "OPENFO4_SKIP_MENU";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    Menu,
    Direct,
}

impl StartMode {
    /// The dev and capture overrides that name where to start (or ask to skip the menu) keep
    /// the direct start; without one the menu opens.
    pub fn from_overrides(spawn_set: bool, spawn_cell_set: bool, skip_menu: bool) -> Self {
        if spawn_set || spawn_cell_set || skip_menu {
            Self::Direct
        } else {
            Self::Menu
        }
    }

    pub fn from_env() -> Self {
        Self::from_overrides(
            std::env::var_os(SPAWN_VAR).is_some(),
            std::env::var_os(SPAWN_CELL_VAR).is_some(),
            crate::utils::env_flag(SKIP_MENU_VAR, false)
                || std::env::var_os(CAPTURE_DIR_ENV).is_some()
                || shot_requested(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_overrides_the_menu_opens() {
        assert_eq!(
            StartMode::from_overrides(false, false, false),
            StartMode::Menu
        );
    }

    #[test]
    fn each_override_alone_starts_directly() {
        for (spawn, cell, skip) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert_eq!(
                StartMode::from_overrides(spawn, cell, skip),
                StartMode::Direct
            );
        }
    }
}
