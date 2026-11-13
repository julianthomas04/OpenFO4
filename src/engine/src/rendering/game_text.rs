//! Texts the engine reads from game settings, and the one rule for when a text counts as present.

use std::sync::Arc;

use anyhow::Context;

use crate::assets::db::DbClient;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum SettingTextError {
    #[error("game setting {setting} is empty")]
    Empty { setting: &'static str },
}

/// The text when it holds more than whitespace.
pub(super) fn non_blank(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.trim().is_empty())
}

/// The text of the setting `setting`, which must hold more than whitespace.
pub(super) fn setting_text(
    text: &str,
    setting: &'static str,
) -> Result<Arc<str>, SettingTextError> {
    non_blank(Some(text))
        .map(Arc::from)
        .ok_or(SettingTextError::Empty { setting })
}

/// The text of a game setting the engine cannot run without; a missing setting is an error.
pub(super) fn required_setting_text(db: &DbClient, name: &'static str) -> anyhow::Result<String> {
    db.get_game_setting_string(name)?
        .with_context(|| format!("game setting {name} is missing from the database"))
}

/// The text of a game setting that must exist and hold more than whitespace.
pub(super) fn required_nonblank_text(
    db: &DbClient,
    name: &'static str,
) -> anyhow::Result<Arc<str>> {
    Ok(setting_text(&required_setting_text(db, name)?, name)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_text_with_a_visible_character_is_present() {
        assert_eq!(non_blank(Some("Take")), Some("Take"));
        for text in [Some(""), Some(" \t\n"), None] {
            assert_eq!(non_blank(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_setting_text_keeps_its_text_verbatim_and_a_blank_one_names_the_setting() {
        assert_eq!(setting_text(" Take ", "sTake").unwrap().as_ref(), " Take ");
        assert_eq!(
            setting_text("  ", "sTake").err(),
            Some(SettingTextError::Empty { setting: "sTake" })
        );
    }
}
