//! The requirement line under the info panel's stars.

use crate::sim::pipboy::perks::{PerkChartSlot, PerkRank};

/// Stand-in: the game builds this line in native code and its exact wording is not in the
/// interface files, so the line names the level and the SPECIAL rank the rank asks for, in the
/// layout's own words. Replace the body once the native format is known.
pub fn requirement_line(
    level_word: &str,
    special_label: &str,
    rank: &PerkRank,
    slot: Option<PerkChartSlot>,
) -> String {
    let level = format!("{level_word} {}", rank.required_level);
    match slot {
        Some(slot) => format!("{level}   {special_label} {}", slot.required_special),
        None => level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::Special;

    fn rank(level: u32) -> PerkRank {
        PerkRank {
            form_key: String::new(),
            description: String::new(),
            required_level: level,
        }
    }

    #[test]
    fn the_line_names_the_level_and_the_special_rank() {
        let slot = PerkChartSlot {
            special: Special::Strength,
            required_special: 3,
        };
        assert_eq!(
            requirement_line("level", "attribute", &rank(12), Some(slot)),
            "level 12   attribute 3"
        );
    }

    #[test]
    fn a_perk_off_the_chart_names_only_its_level() {
        assert_eq!(
            requirement_line("level", "attribute", &rank(4), None),
            "level 4"
        );
    }
}
