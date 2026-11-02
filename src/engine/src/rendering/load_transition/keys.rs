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

/// What a key event does to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::rendering) enum ArtEffect {
    Pan { key: PanKey, held: bool },
    ToggleColours,
}

impl ArtKey {
    /// The effect of `key` going down or up. Key repeats do nothing: a pan key stays held until
    /// its release, and a repeating colour key must not switch twice.
    pub(in crate::rendering) fn effect(self, pressed: bool, repeat: bool) -> Option<ArtEffect> {
        match self {
            _ if repeat => None,
            Self::Pan(key) => Some(ArtEffect::Pan { key, held: pressed }),
            Self::ToggleColours if pressed => Some(ArtEffect::ToggleColours),
            Self::ToggleColours => None,
        }
    }
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

/// The model key `code` is, when the keys act on the model at all: while `interactive` is false
/// every key belongs to the menu.
pub(in crate::rendering) fn routed_art_key(code: KeyCode, interactive: bool) -> Option<ArtKey> {
    art_key(code).filter(|_| interactive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_key_is_left_to_the_menu_unless_the_keys_act_on_the_model() {
        assert_eq!(
            routed_art_key(KeyCode::KeyW, true),
            Some(ArtKey::Pan(PanKey::Up))
        );
        assert_eq!(routed_art_key(KeyCode::KeyW, false), None);
        assert_eq!(routed_art_key(KeyCode::KeyQ, false), None);
        assert_eq!(routed_art_key(KeyCode::Enter, true), None);
    }

    #[test]
    fn a_pan_key_is_held_from_its_press_to_its_release() {
        let pan = ArtKey::Pan(PanKey::Left);
        assert_eq!(
            pan.effect(true, false),
            Some(ArtEffect::Pan {
                key: PanKey::Left,
                held: true
            })
        );
        assert_eq!(
            pan.effect(false, false),
            Some(ArtEffect::Pan {
                key: PanKey::Left,
                held: false
            })
        );
    }

    #[test]
    fn the_colour_key_switches_once_per_press_and_not_on_release() {
        assert_eq!(
            ArtKey::ToggleColours.effect(true, false),
            Some(ArtEffect::ToggleColours)
        );
        assert_eq!(ArtKey::ToggleColours.effect(false, false), None);
    }

    #[test]
    fn a_key_repeat_does_nothing() {
        for key in [ArtKey::Pan(PanKey::Down), ArtKey::ToggleColours] {
            for pressed in [true, false] {
                assert_eq!(key.effect(pressed, true), None, "{key:?} {pressed}");
            }
        }
    }

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
