//! The text-setting rule of the game's shared helper (`GlobalFunc.SetText`): empty text becomes a
//! single space, text not starting with the translate marker is optionally upper-cased, and the
//! shown text is optionally truncated with an ellipsis. Text that starts with the marker is a
//! translate key, which the game's native text hook replaces on assignment; Scaleform leaves a
//! key with no translation as it is, so runtime text such as "$5 Cap" passes through unchanged.
//! This Rust implementation re-creates observed runtime behavior using constants extracted
//! from the installed game's movie; it does not copy the movie's `ActionScript` implementation.
//!
//! Only plain-text assignment is modelled; no consumer sets html text through the rule.

use super::error::UiLayoutError;
use super::keys::{Declaration, KeyCheck, LiteralKey, NumberKey};
use super::layout::UiLayout;

/// The keys of the rule in one movie; declare one per movie whose text goes through it.
pub struct SetTextKeys {
    max_length: NumberKey,
    ellipsis_overhang: LiteralKey,
    ellipsis: LiteralKey,
    slice_start: LiteralKey,
    marker_index: LiteralKey,
    marker: LiteralKey,
    empty_text: LiteralKey,
    empty_replacement: LiteralKey,
}

impl SetTextKeys {
    pub const fn new(movie: &'static str) -> Self {
        Self {
            max_length: NumberKey::new(movie, "Shared.GlobalFunc.MAX_TRUNCATED_TEXT_LENGTH"),
            ellipsis_overhang: LiteralKey::new(
                movie,
                "Shared.GlobalFunc.SetText#MAX_TRUNCATED_TEXT_LENGTH-",
                1,
            ),
            ellipsis: LiteralKey::texts(movie, "Shared.GlobalFunc.SetText#$1.text.slice()+", 1),
            slice_start: LiteralKey::new(movie, "Shared.GlobalFunc.SetText#$1.text.slice(0)", 1),
            marker_index: LiteralKey::new(movie, "Shared.GlobalFunc.SetText#$2.charAt(0)", 1),
            marker: LiteralKey::texts(movie, "Shared.GlobalFunc.SetText#$2.charAt()==", 1),
            empty_replacement: LiteralKey::texts(movie, "Shared.GlobalFunc.SetText#$2=", 1),
            empty_text: LiteralKey::texts(movie, "Shared.GlobalFunc.SetText#$2==", 1),
        }
    }

    fn literal_keys(&self) -> [&LiteralKey; 7] {
        [
            &self.ellipsis_overhang,
            &self.ellipsis,
            &self.slice_start,
            &self.marker_index,
            &self.marker,
            &self.empty_text,
            &self.empty_replacement,
        ]
    }

    pub fn resolve(&self, layout: &UiLayout) -> Result<SetTextRules, UiLayoutError> {
        let whole_literal = |key: &LiteralKey| Ok(layout.lit_whole(key, 0)? as usize);
        Ok(SetTextRules {
            max_length: layout.whole_constant(&self.max_length)? as usize,
            ellipsis_overhang: whole_literal(&self.ellipsis_overhang)?,
            ellipsis: layout.lit_string(&self.ellipsis, 0)?,
            slice_start: whole_literal(&self.slice_start)?,
            marker_index: whole_literal(&self.marker_index)?,
            marker: layout.lit_string(&self.marker, 0)?,
            empty_text: layout.lit_string(&self.empty_text, 0)?,
            empty_replacement: layout.lit_string(&self.empty_replacement, 0)?,
        })
    }
}

impl KeyCheck for SetTextKeys {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let mut failures = self.max_length.failures(layout);
        for key in self.literal_keys() {
            failures.extend(key.failures(layout));
        }
        if failures.is_empty() {
            failures.extend(self.resolve(layout).err());
        }
        failures
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        self.max_length.declare(out);
        for key in self.literal_keys() {
            key.declare(out);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTextRules {
    max_length: usize,
    ellipsis_overhang: usize,
    ellipsis: String,
    slice_start: usize,
    marker_index: usize,
    marker: String,
    empty_text: String,
    empty_replacement: String,
}

impl SetTextRules {
    /// The text a field shows after `SetText(field, text, _, to_upper, truncate)` and the native
    /// translation of a marked text.
    pub fn apply(&self, layout: &UiLayout, text: &str, to_upper: bool, truncate: bool) -> String {
        let text = if text == self.empty_text {
            self.empty_replacement.as_str()
        } else {
            text
        };
        let shown = if self.starts_with_marker(text) {
            self.translated(layout, text)
        } else if to_upper {
            upper_case(text)
        } else {
            text.to_owned()
        };
        if truncate {
            self.truncated(&shown)
        } else {
            shown
        }
    }

    /// A marked text as the native hook shows it. A doubled marker marks an embedded key: the
    /// first word after one marker is translated and the rest kept ("$$KEY (3)"). Otherwise
    /// the whole text is the key. A key the table lacks stays as written, as Scaleform leaves it.
    fn translated(&self, layout: &UiLayout, text: &str) -> String {
        let embedded = text
            .strip_prefix(self.marker.as_str())
            .filter(|rest| rest.starts_with(self.marker.as_str()));
        let Some(rest) = embedded else {
            return translated_key(layout, text);
        };
        let (key, tail) = rest.split_at(rest.find(' ').unwrap_or(rest.len()));
        format!("{}{tail}", translated_key(layout, key))
    }

    fn starts_with_marker(&self, text: &str) -> bool {
        let unit = text.encode_utf16().nth(self.marker_index);
        let marker: Vec<u16> = self.marker.encode_utf16().collect();
        unit.is_some_and(|unit| marker == [unit])
    }

    /// Text longer than the limit (in UTF-16 units, as the game counts) cut to the limit minus
    /// the overhang from the slice start, followed by the ellipsis. Like the game, the cut may
    /// split a surrogate pair.
    pub fn truncated(&self, text: &str) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.len() <= self.max_length {
            return text.to_owned();
        }
        let end = self.max_length.saturating_sub(self.ellipsis_overhang);
        let kept = units
            .get(self.slice_start.min(end)..end)
            .unwrap_or_default();
        let mut result = String::from_utf16_lossy(kept);
        result.push_str(&self.ellipsis);
        result
    }
}

fn translated_key(layout: &UiLayout, key: &str) -> String {
    match layout.translate_text(key) {
        Ok(text) => text,
        Err(UiLayoutError::MissingKey { .. }) => key.to_owned(),
        Err(error) => {
            crate::utils::warn_once(&format!("ui-text-untranslated:{key}"), || {
                format!("UI text '{key}' is shown untranslated: {error}")
            });
            key.to_owned()
        }
    }
}

/// `String.toUpperCase` with simple case mappings: every character maps to one character, so
/// the text keeps its length ("ß" stays "ß").
pub fn upper_case(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => c,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{
        FIXTURE_ELLIPSIS, FIXTURE_ELLIPSIS_OVERHANG, FIXTURE_TEXT_LIMIT, layout_with_movie,
        set_text_movie,
    };

    const KEPT: usize = FIXTURE_TEXT_LIMIT - FIXTURE_ELLIPSIS_OVERHANG;

    const MOVIE: &str = "Fixture";
    static KEYS: SetTextKeys = SetTextKeys::new(MOVIE);

    fn rules() -> (UiLayout, SetTextRules) {
        let layout = layout_with_movie(MOVIE, &set_text_movie());
        let rules = KEYS.resolve(&layout).unwrap();
        (layout, rules)
    }

    #[test]
    fn long_text_is_cut_to_the_limit_minus_the_overhang_with_an_ellipsis() {
        let (layout, rules) = rules();
        let long = "a".repeat(FIXTURE_TEXT_LIMIT + 8);
        let shown = rules.apply(&layout, &long, false, true);
        assert_eq!(shown, format!("{}{FIXTURE_ELLIPSIS}", "a".repeat(KEPT)));
        assert_eq!(rules.apply(&layout, &long, false, false), long);
    }

    #[test]
    fn text_at_the_limit_is_kept() {
        let (layout, rules) = rules();
        let exact = "b".repeat(FIXTURE_TEXT_LIMIT);
        assert_eq!(rules.apply(&layout, &exact, false, true), exact);
    }

    #[test]
    fn empty_text_becomes_one_space() {
        let (layout, rules) = rules();
        assert_eq!(rules.apply(&layout, "", false, false), " ");
    }

    #[test]
    fn upper_casing_applies_to_plain_text() {
        let (layout, rules) = rules();
        assert_eq!(rules.apply(&layout, "widget", true, false), "WIDGET");
    }

    #[test]
    fn a_dollar_key_is_translated_and_not_upper_cased() {
        let (layout, rules) = rules();
        assert_eq!(
            rules.apply(&layout, "$FIXTURE_A", true, false),
            "Fixture alpha"
        );
    }

    #[test]
    fn runtime_text_with_an_unknown_key_passes_through_unchanged() {
        let (layout, rules) = rules();
        assert_eq!(rules.apply(&layout, "$12 units", true, false), "$12 units");
    }

    #[test]
    fn a_doubled_marker_translates_the_first_word_and_keeps_the_rest() {
        let (layout, rules) = rules();
        assert_eq!(
            rules.apply(&layout, "$$FIXTURE_A (3)", false, false),
            "Fixture alpha (3)"
        );
        assert_eq!(
            rules.apply(&layout, "$$NOPE (3)", false, false),
            "$NOPE (3)"
        );
    }

    #[test]
    fn upper_casing_keeps_one_character_per_character() {
        assert_eq!(upper_case("straße"), "STRAßE");
        assert_eq!(upper_case("élan"), "ÉLAN");
    }

    #[test]
    fn truncation_counts_utf16_units_and_may_split_a_surrogate_pair_like_the_game() {
        let (_, rules) = rules();
        let text = format!("{}\u{1F600}{}", "a".repeat(KEPT - 1), "b".repeat(10));
        let shown = rules.truncated(&text);
        assert!(shown.starts_with(&"a".repeat(KEPT - 1)));
        assert!(shown.ends_with(FIXTURE_ELLIPSIS));
        let ellipsis_units = FIXTURE_ELLIPSIS.encode_utf16().count();
        assert_eq!(shown.encode_utf16().count(), KEPT + ellipsis_units);
    }

    #[test]
    fn a_movie_without_the_rule_fails_validation_naming_the_keys() {
        let layout = layout_with_movie(MOVIE, &crate::ui_layout::test_support::empty_movie());
        let error = layout.validate(&[&KEYS]).unwrap_err();
        assert!(error.to_string().contains("MAX_TRUNCATED_TEXT_LENGTH"));
    }
}
