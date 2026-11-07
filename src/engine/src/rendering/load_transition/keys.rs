//! The keys that act on a large loading screen's model while it is up.

use winit::keyboard::KeyCode;

use crate::rendering::loading_screen::PanKey;

/// The pan key `code` is, if it is one: W, A, S and D move the model up, left, down and right on
/// the screen.
pub(in crate::rendering) fn art_key(code: KeyCode) -> Option<PanKey> {
    match code {
        KeyCode::KeyW => Some(PanKey::Up),
        KeyCode::KeyS => Some(PanKey::Down),
        KeyCode::KeyA => Some(PanKey::Left),
        KeyCode::KeyD => Some(PanKey::Right),
        _ => None,
    }
}

/// The pan key `code` is, when the keys act on the model at all: while `interactive` is false
/// every key belongs to the menu.
pub(in crate::rendering) fn routed_art_key(code: KeyCode, interactive: bool) -> Option<PanKey> {
    art_key(code).filter(|_| interactive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_key_is_left_to_the_menu_unless_the_keys_act_on_the_model() {
        assert_eq!(routed_art_key(KeyCode::KeyW, true), Some(PanKey::Up));
        assert_eq!(routed_art_key(KeyCode::KeyW, false), None);
        assert_eq!(routed_art_key(KeyCode::Enter, true), None);
    }

    #[test]
    fn the_walk_keys_pan_the_model_the_way_they_point() {
        assert_eq!(art_key(KeyCode::KeyW), Some(PanKey::Up));
        assert_eq!(art_key(KeyCode::KeyS), Some(PanKey::Down));
        assert_eq!(art_key(KeyCode::KeyA), Some(PanKey::Left));
        assert_eq!(art_key(KeyCode::KeyD), Some(PanKey::Right));
    }

    #[test]
    fn no_other_key_acts_on_the_model() {
        for code in [
            KeyCode::KeyQ,
            KeyCode::KeyE,
            KeyCode::KeyV,
            KeyCode::ArrowUp,
            KeyCode::Space,
            KeyCode::Enter,
            KeyCode::Escape,
        ] {
            assert_eq!(art_key(code), None, "{code:?}");
        }
    }
}
