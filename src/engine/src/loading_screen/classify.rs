//! The kind of loading screen a transition gets. This is the one place the rule lives.

/// Where the player stands before or after a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Interior,
    Exterior { worldspace: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    /// No loading screen at all.
    None,
    /// The black screen with the animated logo.
    Small,
    /// A loading screen with a 3D model and a tip.
    Large,
}

/// `origin` is `None` for the initial load. No public source documents the vanilla rule; this one
/// is inferred: a large screen for the initial load and for a change of worldspace, a small one
/// for everything else, and none for a load door flagged to skip its screen. Form keys, worldspaces
/// included, compare without regard to case.
pub fn classify(origin: Option<&Place>, destination: &Place, no_load_screen: bool) -> LoadKind {
    let Some(origin) = origin else {
        return LoadKind::Large;
    };
    if no_load_screen {
        return LoadKind::None;
    }
    match (origin, destination) {
        (Place::Exterior { worldspace: from }, Place::Exterior { worldspace: to })
            if !from.eq_ignore_ascii_case(to) =>
        {
            LoadKind::Large
        }
        _ => LoadKind::Small,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exterior(worldspace: &str) -> Place {
        Place::Exterior {
            worldspace: worldspace.to_string(),
        }
    }

    #[test]
    fn the_initial_load_is_large_wherever_the_player_arrives() {
        for destination in [Place::Interior, exterior("Commonwealth")] {
            assert_eq!(classify(None, &destination, false), LoadKind::Large);
        }
    }

    #[test]
    fn the_initial_load_is_large_even_through_a_door_that_skips_its_screen() {
        assert_eq!(classify(None, &Place::Interior, true), LoadKind::Large);
    }

    #[test]
    fn a_door_that_skips_its_screen_shows_none_on_every_later_transition() {
        let places = [
            Place::Interior,
            exterior("Commonwealth"),
            exterior("Diamond City"),
        ];
        for origin in &places {
            for destination in &places {
                assert_eq!(
                    classify(Some(origin), destination, true),
                    LoadKind::None,
                    "{origin:?} to {destination:?}"
                );
            }
        }
    }

    #[test]
    fn a_change_of_worldspace_is_large() {
        assert_eq!(
            classify(
                Some(&exterior("Commonwealth")),
                &exterior("Diamond City"),
                false
            ),
            LoadKind::Large
        );
    }

    #[test]
    fn worldspace_form_keys_compare_without_regard_to_case() {
        assert_eq!(
            classify(
                Some(&exterior("003C:Fallout4.esm")),
                &exterior("003c:fallout4.esm"),
                false
            ),
            LoadKind::Small
        );
    }

    #[test]
    fn transitions_within_a_worldspace_or_through_interiors_are_small() {
        let cases = [
            (Place::Interior, Place::Interior),
            (Place::Interior, exterior("Commonwealth")),
            (exterior("Commonwealth"), Place::Interior),
            (exterior("Commonwealth"), exterior("Commonwealth")),
        ];
        for (origin, destination) in cases {
            assert_eq!(
                classify(Some(&origin), &destination, false),
                LoadKind::Small,
                "{origin:?} to {destination:?}"
            );
        }
    }
}
