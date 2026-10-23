//! Resolves a window key event to the key code the game acts on.

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

/// The key code of an event. The arrow keys are resolved by their logical key: input injected
/// without an extended scancode (a keystroke sent by another program, or the numpad's arrow
/// layer) reports the numpad digit's physical key for the same arrow.
pub fn resolve_key_code(physical: PhysicalKey, logical: &Key) -> Option<KeyCode> {
    match logical {
        Key::Named(NamedKey::ArrowUp) => Some(KeyCode::ArrowUp),
        Key::Named(NamedKey::ArrowDown) => Some(KeyCode::ArrowDown),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyCode::ArrowLeft),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCode::ArrowRight),
        _ => match physical {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NativeKey;
    use winit::keyboard::NativeKeyCode;

    #[test]
    fn an_arrow_reported_as_a_numpad_digit_resolves_to_the_arrow() {
        let cases = [
            (NamedKey::ArrowUp, KeyCode::Numpad8, KeyCode::ArrowUp),
            (NamedKey::ArrowDown, KeyCode::Numpad2, KeyCode::ArrowDown),
            (NamedKey::ArrowLeft, KeyCode::Numpad4, KeyCode::ArrowLeft),
            (NamedKey::ArrowRight, KeyCode::Numpad6, KeyCode::ArrowRight),
        ];
        for (named, digit, arrow) in cases {
            let resolved = resolve_key_code(PhysicalKey::Code(digit), &Key::Named(named));
            assert_eq!(resolved, Some(arrow));
        }
    }

    #[test]
    fn an_arrow_without_a_physical_key_resolves_to_the_arrow() {
        let physical = PhysicalKey::Unidentified(NativeKeyCode::Windows(0));
        let resolved = resolve_key_code(physical, &Key::Named(NamedKey::ArrowDown));
        assert_eq!(resolved, Some(KeyCode::ArrowDown));
    }

    #[test]
    fn a_numpad_digit_with_a_digit_logical_key_stays_a_digit() {
        let resolved = resolve_key_code(
            PhysicalKey::Code(KeyCode::Numpad2),
            &Key::Character("2".into()),
        );
        assert_eq!(resolved, Some(KeyCode::Numpad2));
    }

    #[test]
    fn other_keys_resolve_by_their_physical_key() {
        let resolved = resolve_key_code(
            PhysicalKey::Code(KeyCode::KeyS),
            &Key::Character("s".into()),
        );
        assert_eq!(resolved, Some(KeyCode::KeyS));
    }

    #[test]
    fn an_unidentified_non_arrow_key_resolves_to_nothing() {
        let physical = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        let logical = Key::Unidentified(NativeKey::Unidentified);
        assert_eq!(resolve_key_code(physical, &logical), None);
    }
}
