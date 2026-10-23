//! Perk definitions (from the database) and the ranks the player owns.

use std::collections::HashMap;

use super::character::{Sex, Special};
use super::{PipBoyActionError, PipBoyModel};

/// A perk, identified by the form key of its first-rank record.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PerkId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct PerkRank {
    pub form_key: String,
    pub description: String,
    pub required_level: u32,
}

/// Where a perk sits on the perk chart: its SPECIAL column and the attribute rank (row `1..=10`)
/// that unlocks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerkChartSlot {
    pub special: Special,
    pub required_special: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerkDefinition {
    pub id: PerkId,
    pub name: String,
    /// Ranks in order; `ranks[0]` is the record named by `id`.
    pub ranks: Vec<PerkRank>,
    /// Raw `Interface`-relative path of the perk's Vault Boy movie, as stored in the database.
    pub vault_boy_swf: Option<String>,
    /// `None` for perks outside the chart (magazine, companion, faction perks).
    pub chart: Option<PerkChartSlot>,
    /// `Some` for the perks that share a chart cell with a counterpart of the other sex.
    pub sex: Option<Sex>,
}

impl PerkDefinition {
    /// Whether a character of `sex` can see and take the perk.
    pub fn is_available_to(&self, sex: Sex) -> bool {
        self.sex.is_none_or(|own| own == sex)
    }
}

/// Owned rank per perk, `1..=ranks.len()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnedPerks(pub HashMap<PerkId, u8>);

/// What the player can do with a perk right now; drives rank stars and the perk chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerkAvailability {
    /// Rank `owned` is held and the next rank can be taken now.
    CanRankUp { owned: u8 },
    /// Rank `owned` is held (`0` = none) and the next rank is blocked.
    Locked { owned: u8 },
    /// Every rank is held.
    Maxed,
}

impl PipBoyModel {
    pub fn perk_definition(&self, perk: &PerkId) -> Option<&PerkDefinition> {
        self.catalog.perks.iter().find(|def| &def.id == perk)
    }

    /// `None` for a perk the catalog does not know. `CanRankUp` includes holding an unspent point.
    pub fn perk_availability(&self, perk: &PerkId) -> Option<PerkAvailability> {
        let owned = self.perks.rank(perk);
        match self.check_rank_up(perk) {
            Ok(()) => Some(PerkAvailability::CanRankUp { owned }),
            Err(PipBoyActionError::UnknownPerk(_)) => None,
            Err(PipBoyActionError::PerkAtMaxRank(_)) => Some(PerkAvailability::Maxed),
            Err(_) => Some(PerkAvailability::Locked { owned }),
        }
    }

    /// Why the next rank of `perk` cannot be taken now, if it cannot.
    pub(super) fn check_rank_up(&self, perk: &PerkId) -> Result<(), PipBoyActionError> {
        let (definition, next_rank) = self.next_rank_of(perk)?;
        if self.character.progression.unspent_perk_points == 0 {
            return Err(PipBoyActionError::NoPerkPoints);
        }
        self.check_requirements(perk, definition, next_rank)
    }

    /// Whether the next rank of `perk` meets its level and SPECIAL requirements, whatever the
    /// unspent points; `false` for an unknown or maxed perk.
    pub fn perk_next_rank_requirements_met(&self, perk: &PerkId) -> bool {
        self.next_rank_of(perk)
            .is_ok_and(|(definition, next_rank)| {
                self.check_requirements(perk, definition, next_rank).is_ok()
            })
    }

    fn next_rank_of(
        &self,
        perk: &PerkId,
    ) -> Result<(&PerkDefinition, &PerkRank), PipBoyActionError> {
        let definition = self
            .perk_definition(perk)
            .filter(|definition| definition.is_available_to(self.character.sex))
            .ok_or_else(|| PipBoyActionError::UnknownPerk(perk.clone()))?;
        let owned = usize::from(self.perks.rank(perk));
        let next_rank = definition
            .ranks
            .get(owned)
            .ok_or_else(|| PipBoyActionError::PerkAtMaxRank(perk.clone()))?;
        Ok((definition, next_rank))
    }

    fn check_requirements(
        &self,
        perk: &PerkId,
        definition: &PerkDefinition,
        next_rank: &PerkRank,
    ) -> Result<(), PipBoyActionError> {
        if self.character.progression.level < next_rank.required_level {
            return Err(PipBoyActionError::PerkLevelTooLow {
                perk: perk.clone(),
                required_level: next_rank.required_level,
            });
        }
        if let Some(slot) = definition.chart
            && self.character.special.get(slot.special).base < slot.required_special
        {
            return Err(PipBoyActionError::PerkSpecialTooLow { perk: perk.clone() });
        }
        Ok(())
    }
}

impl OwnedPerks {
    pub fn rank(&self, perk: &PerkId) -> u8 {
        self.0.get(perk).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{catalog_with, gun_nut, perk, perk_model};
    use super::*;

    #[test]
    fn an_unknown_perk_has_no_availability() {
        assert_eq!(
            perk_model().perk_availability(&PerkId("Nope".to_owned())),
            None
        );
    }

    #[test]
    fn a_perk_meeting_every_requirement_can_rank_up() {
        assert_eq!(
            perk_model().perk_availability(&gun_nut()),
            Some(PerkAvailability::CanRankUp { owned: 0 })
        );
    }

    #[test]
    fn without_a_point_the_perk_is_locked() {
        let mut model = perk_model();
        model.character.progression.unspent_perk_points = 0;
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::Locked { owned: 0 })
        );
    }

    #[test]
    fn a_low_level_locks_the_next_rank_only() {
        let mut model = perk_model();
        model.perks.0.insert(gun_nut(), 1);
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::Locked { owned: 1 })
        );
        model.character.progression.level = 15;
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::CanRankUp { owned: 1 })
        );
    }

    #[test]
    fn the_chart_row_is_compared_with_the_base_rank_at_every_boundary() {
        for (base, can_rank_up) in [(1, false), (2, false), (3, true), (4, true), (10, true)] {
            let mut model = perk_model();
            model.character.special.0[Special::Intelligence as usize].base = base;
            let expected = if can_rank_up {
                PerkAvailability::CanRankUp { owned: 0 }
            } else {
                PerkAvailability::Locked { owned: 0 }
            };
            assert_eq!(
                model.perk_availability(&gun_nut()),
                Some(expected),
                "INT {base}"
            );
        }
    }

    #[test]
    fn each_rank_is_gated_by_its_own_level() {
        for (level, expected) in [
            (4, PerkAvailability::Locked { owned: 0 }),
            (5, PerkAvailability::CanRankUp { owned: 0 }),
        ] {
            let mut model = perk_model();
            model.character.progression.level = level;
            assert_eq!(
                model.perk_availability(&gun_nut()),
                Some(expected),
                "level {level}"
            );
        }
        for (level, expected) in [
            (14, PerkAvailability::Locked { owned: 1 }),
            (15, PerkAvailability::CanRankUp { owned: 1 }),
        ] {
            let mut model = perk_model();
            model.perks.0.insert(gun_nut(), 1);
            model.character.progression.level = level;
            assert_eq!(
                model.perk_availability(&gun_nut()),
                Some(expected),
                "level {level}"
            );
        }
    }

    #[test]
    fn a_second_perk_is_judged_against_its_own_attribute() {
        let savant = PerkId("Savant:Fallout4.esm".to_owned());
        let mut model = perk_model();
        model.character.special.0[Special::Luck as usize].base = 0;
        assert_eq!(
            model.perk_availability(&savant),
            Some(PerkAvailability::Locked { owned: 0 })
        );
        model.character.special.0[Special::Luck as usize].base = 1;
        assert_eq!(
            model.perk_availability(&savant),
            Some(PerkAvailability::CanRankUp { owned: 0 })
        );
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::CanRankUp { owned: 0 }),
            "the other perk is unaffected"
        );
    }

    #[test]
    fn a_low_special_locks_a_chart_perk() {
        let mut model = perk_model();
        model.character.special.0[Special::Intelligence as usize].base = 1;
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::Locked { owned: 0 })
        );
    }

    #[test]
    fn a_temporary_special_bonus_does_not_satisfy_the_chart() {
        let mut model = perk_model();
        model.character.special.0[Special::Intelligence as usize].base = 1;
        model.character.special.0[Special::Intelligence as usize].bonus = 5;
        assert!(matches!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::Locked { .. })
        ));
    }

    #[test]
    fn a_perk_of_the_other_sex_is_unknown_to_the_character() {
        let mut gendered = perk("Widow", Special::Charisma, 1, &[1]);
        gendered.sex = Some(Sex::Female);
        let mut model = PipBoyModel::new(catalog_with(vec![gendered], Vec::new()));
        model.character.progression.unspent_perk_points = 1;
        let id = PerkId("Widow:Fallout4.esm".to_owned());
        assert_eq!(model.perk_availability(&id), None);
        model.character.sex = Sex::Female;
        assert_eq!(
            model.perk_availability(&id),
            Some(PerkAvailability::CanRankUp { owned: 0 })
        );
    }

    #[test]
    fn owning_every_rank_is_maxed() {
        let mut model = perk_model();
        model.perks.0.insert(gun_nut(), 2);
        assert_eq!(
            model.perk_availability(&gun_nut()),
            Some(PerkAvailability::Maxed)
        );
    }
}
