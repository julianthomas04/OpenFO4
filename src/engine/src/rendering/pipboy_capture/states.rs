//! The fixed list of Pip-Boy screens to photograph and how to reach each.

use crate::rendering::pipboy::input::ShellInput;
use crate::rendering::pipboy::nav::{Page, Screen, SubTab, TabId};
use crate::rendering::pipboy::tab::{HintKey, NavInput};

/// The list row selected in the selection state of each list page.
pub const SELECTED_ROW: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptureAction {
    Show {
        screen: Screen,
        select_row: Option<usize>,
    },
    Input(ShellInput),
}

/// What the engine has to be showing before a state can be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    MainMenu,
    /// The world with the HUD and the Pip-Boy closed.
    Hud,
    PipBoy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureState {
    pub name: String,
    pub scene: Scene,
    pub actions: Vec<CaptureAction>,
}

impl CaptureState {
    fn show(name: impl Into<String>, screen: Screen, select_row: Option<usize>) -> Self {
        Self {
            name: name.into(),
            scene: Scene::PipBoy,
            actions: vec![CaptureAction::Show { screen, select_row }],
        }
    }

    fn plain(name: &str, scene: Scene) -> Self {
        Self {
            name: name.to_owned(),
            scene,
            actions: Vec::new(),
        }
    }

    pub fn file_name(&self) -> String {
        format!("{}.png", self.name)
    }
}

fn tab_name(page: Page, sub: Option<SubTab>) -> String {
    let page = page.label().to_ascii_lowercase();
    match sub {
        Some(sub) => format!("{page}_{}", sub.label().to_ascii_lowercase()),
        None => page,
    }
}

/// Whether the tab is a list whose rows can be selected.
fn has_rows(tab: TabId) -> bool {
    !matches!(tab, TabId::Map | TabId::Sub(SubTab::Status))
}

fn page_tabs(page: Page) -> Vec<(Option<SubTab>, TabId)> {
    match page {
        Page::Map => vec![(None, TabId::Map)],
        Page::Radio => vec![(None, TabId::Radio)],
        Page::Stat | Page::Inv | Page::Data => page
            .sub_tabs()
            .iter()
            .map(|&sub| (Some(sub), TabId::Sub(sub)))
            .collect(),
    }
}

fn local_map_state() -> CaptureState {
    CaptureState {
        name: "map_local".to_owned(),
        scene: Scene::PipBoy,
        actions: vec![
            CaptureAction::Show {
                screen: Screen::Shell(TabId::Map),
                select_row: None,
            },
            CaptureAction::Input(ShellInput::Nav(NavInput::Hint(HintKey::R))),
        ],
    }
}

/// The main menu and the HUD, then every page and sub-tab, the selection state of each list, the
/// local map and the perk chart, in the order they are captured.
pub fn capture_states() -> Vec<CaptureState> {
    let mut states = vec![
        CaptureState::plain("main_menu", Scene::MainMenu),
        CaptureState::plain("hud", Scene::Hud),
    ];
    for page in Page::ALL {
        for (sub, tab) in page_tabs(page) {
            let name = if tab == TabId::Map {
                "map_world".to_owned()
            } else {
                tab_name(page, sub)
            };
            let screen = Screen::Shell(tab);
            states.push(CaptureState::show(&name, screen, None));
            if has_rows(tab) {
                states.push(CaptureState::show(
                    format!("{name}_selected"),
                    screen,
                    Some(SELECTED_ROW),
                ));
            }
            if tab == TabId::Map {
                states.push(local_map_state());
            }
        }
    }
    states.push(CaptureState::show("perkchart", Screen::PerkChart, None));
    states
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn names() -> Vec<String> {
        capture_states()
            .into_iter()
            .map(|state| state.name)
            .collect()
    }

    #[test]
    fn every_page_and_sub_tab_has_a_state() {
        let names = names();
        for expected in [
            "stat_status",
            "stat_special",
            "stat_perks",
            "inv_weapons",
            "inv_apparel",
            "inv_aid",
            "inv_misc",
            "inv_junk",
            "inv_mods",
            "inv_ammo",
            "data_quests",
            "data_workshops",
            "data_stats",
            "map_world",
            "map_local",
            "radio",
            "perkchart",
        ] {
            assert!(names.iter().any(|name| name == expected), "{expected}");
        }
    }

    #[test]
    fn list_pages_get_a_selection_state_and_the_others_do_not() {
        let names = names();
        for list in [
            "stat_special",
            "stat_perks",
            "inv_ammo",
            "data_quests",
            "radio",
        ] {
            assert!(names.contains(&format!("{list}_selected")), "{list}");
        }
        for plain in ["stat_status", "map_world", "map_local", "perkchart"] {
            assert!(!names.contains(&format!("{plain}_selected")), "{plain}");
        }
    }

    #[test]
    fn names_are_unique_lowercase_file_stems() {
        let names = names();
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
        for name in &names {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name}"
            );
        }
    }

    #[test]
    fn the_menu_and_the_hud_come_first_and_need_no_actions() {
        let states = capture_states();
        let leading: Vec<_> = states
            .iter()
            .take(2)
            .map(|state| (state.name.as_str(), state.scene, state.actions.is_empty()))
            .collect();
        assert_eq!(
            leading,
            vec![
                ("main_menu", Scene::MainMenu, true),
                ("hud", Scene::Hud, true)
            ]
        );
    }

    #[test]
    fn every_other_state_is_a_pipboy_scene() {
        assert!(
            capture_states()
                .iter()
                .skip(2)
                .all(|state| state.scene == Scene::PipBoy)
        );
    }

    #[test]
    fn file_names_carry_the_png_extension() {
        let state = CaptureState::show("inv_aid", Screen::PerkChart, None);
        assert_eq!(state.file_name(), "inv_aid.png");
    }

    #[test]
    fn selection_states_select_the_fixed_row() {
        let states = capture_states();
        let selected = states
            .iter()
            .find(|state| state.name == "inv_junk_selected")
            .unwrap();
        assert_eq!(
            selected.actions,
            vec![CaptureAction::Show {
                screen: Screen::Shell(TabId::Sub(SubTab::Junk)),
                select_row: Some(SELECTED_ROW),
            }]
        );
    }

    #[test]
    fn the_local_map_is_reached_through_the_map_hint_key() {
        let states = capture_states();
        let local = states
            .iter()
            .find(|state| state.name == "map_local")
            .unwrap();
        assert_eq!(
            local.actions.last(),
            Some(&CaptureAction::Input(ShellInput::Nav(NavInput::Hint(
                HintKey::R
            ))))
        );
    }
}
