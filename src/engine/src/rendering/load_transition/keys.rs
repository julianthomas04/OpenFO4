//! The keys that act on a large loading screen's model while it is up.

use winit::keyboard::KeyCode;

use crate::rendering::loading_screen::PanKey;

/// What a key does to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::rendering) enum ArtKey {
    Pan(PanKey),
    /// Switches between the model's own colours and the HUD colour alone.
    ToggleColours,
}

/// The effect of `code` on the model, if it has one: W, A, S and D move the model up, left, down
/// and right on the screen, and Q switches its colours.
pub(in crate::rendering) fn art_key(code: KeyCode) -> Option<ArtKey> {
    match code {
        KeyCode::KeyW => Some(ArtKey::Pan(PanKey::Up)),
        KeyCode::KeyS => Some(ArtKey::Pan(PanKey::Down)),
        KeyCode::KeyA => Some(ArtKey::Pan(PanKey::Left)),
        KeyCode::KeyD => Some(ArtKey::Pan(PanKey::Right)),
        KeyCode::KeyQ => Some(ArtKey::ToggleColours),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_keys_pan_the_model_the_way_they_point() {
        assert_eq!(art_key(KeyCode::KeyW), Some(ArtKey::Pan(PanKey::Up)));
        assert_eq!(art_key(KeyCode::KeyS), Some(ArtKey::Pan(PanKey::Down)));
        assert_eq!(art_key(KeyCode::KeyA), Some(ArtKey::Pan(PanKey::Left)));
        assert_eq!(art_key(KeyCode::KeyD), Some(ArtKey::Pan(PanKey::Right)));
    }

    #[test]
    fn q_switches_the_colours() {
        assert_eq!(art_key(KeyCode::KeyQ), Some(ArtKey::ToggleColours));
    }

    #[test]
    fn no_other_key_acts_on_the_model() {
        for code in [
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
