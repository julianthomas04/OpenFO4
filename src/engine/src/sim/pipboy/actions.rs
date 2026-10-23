//! Player actions issued from the Pip-Boy and how they mutate the model.

use glam::Vec3;

use super::PipBoyModel;
use super::character::ActiveEffect;
use super::inventory::{
    FAVOURITE_SLOT_COUNT, FavouriteSlot, InventoryCategory, InventoryItem, ItemDetails, ItemId,
    SortMode,
};
use super::map::MapMarkerId;
use super::perks::PerkId;
use super::quests::QuestId;
use super::radio::StationId;

/// Form keys of the items the STATUS page quick-uses.
pub const STIMPAK_FORM_KEY: &str = "023736:Fallout4.esm";
pub const RADAWAY_FORM_KEY: &str = "023742:Fallout4.esm";

#[derive(Clone, Debug, PartialEq)]
pub enum PipBoyAction {
    Open,
    Close,
    Equip(ItemId),
    Unequip(ItemId),
    Use(ItemId),
    UseFirstStimpak,
    UseFirstRadAway,
    Drop {
        item: ItemId,
        count: u32,
    },
    SetFavourite {
        item: ItemId,
        slot: Option<FavouriteSlot>,
    },
    SetSortMode {
        category: InventoryCategory,
        mode: SortMode,
    },
    ToggleComponentSearchTag(String),
    RankUpPerk(PerkId),
    ToggleQuestTracked(QuestId),
    TuneRadio(StationId),
    TurnOffRadio,
    PlaceCustomMarker(Vec3),
    RemoveCustomMarker,
    RequestFastTravel(MapMarkerId),
    /// Asks for the examine view of an item; the model only records the request.
    RequestInspect(ItemId),
}

/// What an accepted action asks the rest of the sim to do; the model itself cannot apply it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionEffects {
    /// Hit points the player receives.
    pub heal_hp: f32,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PipBoyActionError {
    #[error("no inventory item {0:?}")]
    UnknownItem(ItemId),
    #[error("{0:?} cannot be equipped")]
    NotEquippable(ItemId),
    #[error("{0:?} is not equipped")]
    NotEquipped(ItemId),
    #[error("{0:?} cannot be used")]
    NotUsable(ItemId),
    #[error("no item of that kind in the inventory")]
    NoneLeft,
    #[error("cannot act on zero units")]
    ZeroQuantity,
    #[error("map position {0:?} is not finite")]
    NonFinitePosition(Vec3),
    #[error("cannot drop {requested} of {available}")]
    NotEnoughToDrop { requested: u32, available: u32 },
    #[error("favourite slot {0} is out of range")]
    InvalidFavouriteSlot(u8),
    #[error("{mode:?} is not a sort mode of {category:?}")]
    InvalidSortMode {
        category: InventoryCategory,
        mode: SortMode,
    },
    #[error("no perk {0:?}")]
    UnknownPerk(PerkId),
    #[error("no unspent perk points")]
    NoPerkPoints,
    #[error("{0:?} is at its highest rank")]
    PerkAtMaxRank(PerkId),
    #[error("{perk:?} needs level {required_level}")]
    PerkLevelTooLow { perk: PerkId, required_level: u32 },
    #[error("{perk:?} needs a higher SPECIAL rank")]
    PerkSpecialTooLow { perk: PerkId },
    #[error("no quest {0:?}")]
    UnknownQuest(QuestId),
    #[error("no radio station {0:?}")]
    UnknownStation(StationId),
    #[error("{0:?} is out of range")]
    StationOutOfRange(StationId),
    #[error("no map marker {0:?}")]
    UnknownMarker(MapMarkerId),
    #[error("cannot fast travel to {0:?}")]
    CannotFastTravel(MapMarkerId),
}

impl PipBoyActionError {
    /// The translation key of the game's own refusal text for this rejection; `None` where the
    /// game has no text for it (the error then reaches the log, not the player).
    pub fn translation_key(&self) -> Option<&'static str> {
        match self {
            Self::PerkLevelTooLow { .. } | Self::PerkSpecialTooLow { .. } => Some("$PerksLowReqs"),
            Self::UnknownItem(_)
            | Self::NotEquippable(_)
            | Self::NotEquipped(_)
            | Self::NotUsable(_)
            | Self::NoneLeft
            | Self::ZeroQuantity
            | Self::NonFinitePosition(_)
            | Self::NotEnoughToDrop { .. }
            | Self::InvalidFavouriteSlot(_)
            | Self::InvalidSortMode { .. }
            | Self::UnknownPerk(_)
            | Self::NoPerkPoints
            | Self::PerkAtMaxRank(_)
            | Self::UnknownQuest(_)
            | Self::UnknownStation(_)
            | Self::StationOutOfRange(_)
            | Self::UnknownMarker(_)
            | Self::CannotFastTravel(_) => None,
        }
    }
}

impl PipBoyModel {
    /// Applies one action and returns the effects the sim has to carry out. A rejected action
    /// leaves the model unchanged.
    pub fn apply(&mut self, action: PipBoyAction) -> Result<ActionEffects, PipBoyActionError> {
        let mut effects = ActionEffects::default();
        match action {
            PipBoyAction::Open => self.open = true,
            PipBoyAction::Close => self.open = false,
            PipBoyAction::Equip(item) => self.equip(item)?,
            PipBoyAction::Unequip(item) => self.unequip(item)?,
            PipBoyAction::Use(item) => effects = self.use_item(item)?,
            PipBoyAction::UseFirstStimpak => {
                effects = self.use_first_with_form_key(STIMPAK_FORM_KEY)?;
            }
            PipBoyAction::UseFirstRadAway => {
                effects = self.use_first_with_form_key(RADAWAY_FORM_KEY)?;
            }
            PipBoyAction::Drop { item, count } => self.drop_item(item, count)?,
            PipBoyAction::SetFavourite { item, slot } => self.set_favourite(item, slot)?,
            PipBoyAction::SetSortMode { category, mode } => self.set_sort_mode(category, mode)?,
            PipBoyAction::ToggleComponentSearchTag(component) => {
                self.toggle_component_tag(component);
            }
            PipBoyAction::RankUpPerk(perk) => self.rank_up_perk(&perk)?,
            PipBoyAction::ToggleQuestTracked(quest) => self.toggle_quest_tracked(&quest)?,
            PipBoyAction::TuneRadio(station) => self.tune_radio(station)?,
            PipBoyAction::TurnOffRadio => self.radio.tuned = None,
            PipBoyAction::PlaceCustomMarker(position) => self.place_custom_marker(position)?,
            PipBoyAction::RemoveCustomMarker => self.custom_marker = None,
            PipBoyAction::RequestFastTravel(marker) => self.request_fast_travel(marker)?,
            PipBoyAction::RequestInspect(item) => self.request_inspect(item)?,
        }
        Ok(effects)
    }

    /// [`apply`](Self::apply) plus the bookkeeping the sim wants: an accepted action clears
    /// `last_rejection`, a rejected one is stored and logged. A rejected action has no effects.
    pub fn submit(&mut self, action: PipBoyAction) -> ActionEffects {
        match self.apply(action) {
            Ok(effects) => {
                self.last_rejection = None;
                effects
            }
            Err(error) => {
                tracing::warn!("Pip-Boy action rejected: {error}");
                self.last_rejection = Some(error);
                ActionEffects::default()
            }
        }
    }

    fn item(&self, id: ItemId) -> Result<&InventoryItem, PipBoyActionError> {
        self.inventory
            .item(id)
            .ok_or(PipBoyActionError::UnknownItem(id))
    }

    /// Equipped items that must come off for `item` to be worn or wielded.
    fn displaced_by(&self, item: &InventoryItem) -> Result<Vec<ItemId>, PipBoyActionError> {
        let conflicts: fn(&ItemDetails, &ItemDetails) -> bool = match &item.details {
            ItemDetails::Weapon(_) => |_, other| matches!(other, ItemDetails::Weapon(_)),
            ItemDetails::Apparel(stats) if stats.slot_mask == 0 => {
                return Err(PipBoyActionError::NotEquippable(item.id));
            }
            ItemDetails::Apparel(_) => |this, other| match (this, other) {
                (ItemDetails::Apparel(this), ItemDetails::Apparel(other)) => {
                    this.slot_mask & other.slot_mask != 0
                }
                _ => false,
            },
            _ => return Err(PipBoyActionError::NotEquippable(item.id)),
        };
        Ok(self
            .inventory
            .items
            .iter()
            .filter(|other| {
                other.id != item.id && other.equipped && conflicts(&item.details, &other.details)
            })
            .map(|other| other.id)
            .collect())
    }

    fn equip(&mut self, id: ItemId) -> Result<(), PipBoyActionError> {
        let displaced = self.displaced_by(self.item(id)?)?;
        for other in displaced {
            self.set_equipped(other, false);
        }
        self.set_equipped(id, true);
        Ok(())
    }

    fn unequip(&mut self, id: ItemId) -> Result<(), PipBoyActionError> {
        if !self.item(id)?.equipped {
            return Err(PipBoyActionError::NotEquipped(id));
        }
        self.set_equipped(id, false);
        Ok(())
    }

    fn set_equipped(&mut self, id: ItemId, equipped: bool) {
        if let Some(item) = self.inventory.item_mut(id) {
            item.equipped = equipped;
        }
    }

    fn use_first_with_form_key(
        &mut self,
        form_key: &str,
    ) -> Result<ActionEffects, PipBoyActionError> {
        let id = self
            .inventory
            .items
            .iter()
            .find(|item| item.form_key.as_deref() == Some(form_key))
            .map(|item| item.id)
            .ok_or(PipBoyActionError::NoneLeft)?;
        self.use_item(id)
    }

    fn use_item(&mut self, id: ItemId) -> Result<ActionEffects, PipBoyActionError> {
        let item = self.item(id)?;
        let ItemDetails::Aid(effects) = &item.details else {
            return Err(PipBoyActionError::NotUsable(id));
        };
        let healed: f32 = effects.iter().map(|effect| effect.restores_hp).sum();
        let timed: Vec<ActiveEffect> = effects
            .iter()
            .filter(|effect| effect.duration_seconds.is_some())
            .map(|effect| ActiveEffect {
                source: item.name.clone(),
                description: effect.description.clone(),
                remaining_seconds: effect.duration_seconds,
            })
            .collect();
        self.character.effects.extend(timed);
        self.remove_units(id, 1);
        Ok(ActionEffects { heal_hp: healed })
    }

    /// Removes up to `count` units of a validated stack; an emptied stack goes away together
    /// with its equipped and favourite state.
    fn remove_units(&mut self, id: ItemId, count: u32) {
        self.inventory.items.retain_mut(|item| {
            if item.id != id {
                return true;
            }
            item.count = item.count.saturating_sub(count);
            item.count > 0
        });
    }

    fn drop_item(&mut self, id: ItemId, count: u32) -> Result<(), PipBoyActionError> {
        let available = self.item(id)?.count;
        if count == 0 {
            return Err(PipBoyActionError::ZeroQuantity);
        }
        if count > available {
            return Err(PipBoyActionError::NotEnoughToDrop {
                requested: count,
                available,
            });
        }
        self.remove_units(id, count);
        Ok(())
    }

    fn set_favourite(
        &mut self,
        id: ItemId,
        slot: Option<FavouriteSlot>,
    ) -> Result<(), PipBoyActionError> {
        self.item(id)?;
        if let Some(FavouriteSlot(index)) = slot {
            if index >= FAVOURITE_SLOT_COUNT {
                return Err(PipBoyActionError::InvalidFavouriteSlot(index));
            }
            for other in &mut self.inventory.items {
                if other.favourite == slot {
                    other.favourite = None;
                }
            }
        }
        if let Some(item) = self.inventory.item_mut(id) {
            item.favourite = slot;
        }
        Ok(())
    }

    fn set_sort_mode(
        &mut self,
        category: InventoryCategory,
        mode: SortMode,
    ) -> Result<(), PipBoyActionError> {
        if !mode.is_valid_for(category) {
            return Err(PipBoyActionError::InvalidSortMode { category, mode });
        }
        self.inventory.sort[category as usize] = mode;
        Ok(())
    }

    fn toggle_component_tag(&mut self, component: String) {
        let tags = &mut self.inventory.tagged_components;
        match tags.iter().position(|tag| *tag == component) {
            Some(index) => {
                tags.remove(index);
            }
            None => tags.push(component),
        }
    }

    fn rank_up_perk(&mut self, perk: &PerkId) -> Result<(), PipBoyActionError> {
        self.check_rank_up(perk)?;
        *self.perks.0.entry(perk.clone()).or_insert(0) += 1;
        self.character.progression.unspent_perk_points -= 1;
        Ok(())
    }

    fn toggle_quest_tracked(&mut self, id: &QuestId) -> Result<(), PipBoyActionError> {
        let quest = self
            .quests
            .iter_mut()
            .find(|quest| &quest.id == id)
            .ok_or_else(|| PipBoyActionError::UnknownQuest(id.clone()))?;
        quest.tracked = !quest.tracked;
        Ok(())
    }

    fn tune_radio(&mut self, id: StationId) -> Result<(), PipBoyActionError> {
        let station = self
            .radio
            .stations
            .iter()
            .find(|station| station.id == id)
            .ok_or(PipBoyActionError::UnknownStation(id))?;
        if !station.in_range {
            return Err(PipBoyActionError::StationOutOfRange(id));
        }
        self.radio.tuned = Some(id);
        Ok(())
    }

    fn place_custom_marker(&mut self, position: Vec3) -> Result<(), PipBoyActionError> {
        if !position.is_finite() {
            return Err(PipBoyActionError::NonFinitePosition(position));
        }
        self.custom_marker = Some(position);
        Ok(())
    }

    fn request_inspect(&mut self, item: ItemId) -> Result<(), PipBoyActionError> {
        self.inventory
            .item(item)
            .ok_or(PipBoyActionError::UnknownItem(item))?;
        self.inspect_request = Some(item);
        Ok(())
    }

    fn request_fast_travel(&mut self, id: MapMarkerId) -> Result<(), PipBoyActionError> {
        let marker = self
            .catalog
            .world_map
            .as_ref()
            .and_then(|map| map.markers.iter().find(|marker| marker.id == id))
            .ok_or_else(|| PipBoyActionError::UnknownMarker(id.clone()))?;
        if !(self.is_marker_discovered(marker) && marker.can_fast_travel) {
            return Err(PipBoyActionError::CannotFastTravel(id));
        }
        self.fast_travel_request = Some(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::quests::{Quest, QuestStatus};
    use super::super::radio::RadioStation;
    use super::super::test_support::{
        aid, apparel, catalog_with, empty_model, gun_nut, item, marker, perk_model, weapon,
    };
    use super::super::{PipBoyCatalog, Special};
    use super::*;

    fn add(model: &mut PipBoyModel, item: InventoryItem) -> ItemId {
        model.inventory.insert(item)
    }

    #[test]
    fn perk_requirement_rejections_use_the_games_refusal_text_and_the_rest_have_none() {
        let perk = PerkId("perk".to_owned());
        let low_level = PipBoyActionError::PerkLevelTooLow {
            perk: perk.clone(),
            required_level: 5,
        };
        let low_special = PipBoyActionError::PerkSpecialTooLow { perk };

        assert_eq!(low_level.translation_key(), Some("$PerksLowReqs"));
        assert_eq!(low_special.translation_key(), Some("$PerksLowReqs"));
        assert_eq!(PipBoyActionError::NoneLeft.translation_key(), None);
    }

    fn assert_rejected(model: &mut PipBoyModel, action: PipBoyAction, expected: PipBoyActionError) {
        let before = model.clone();
        assert_eq!(model.apply(action), Err(expected));
        assert_eq!(
            *model, before,
            "a rejected action must not change the model"
        );
    }

    fn equipped(model: &PipBoyModel, id: ItemId) -> bool {
        model.inventory.item(id).unwrap().equipped
    }

    #[test]
    fn open_and_close_toggle_the_open_flag() {
        let mut model = empty_model();
        model.apply(PipBoyAction::Open).unwrap();
        assert!(model.open);
        model.apply(PipBoyAction::Close).unwrap();
        assert!(!model.open);
    }

    #[test]
    fn equipping_a_weapon_unequips_the_other_one() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        let rifle = add(&mut model, weapon("Rifle", 20.0, 1.0));
        let vest = add(&mut model, apparel("Vest", 0b1));
        model.apply(PipBoyAction::Equip(vest)).unwrap();
        model.apply(PipBoyAction::Equip(pistol)).unwrap();
        model.apply(PipBoyAction::Equip(rifle)).unwrap();
        assert!(!equipped(&model, pistol));
        assert!(equipped(&model, rifle));
        assert!(
            equipped(&model, vest),
            "apparel is not displaced by a weapon"
        );
    }

    #[test]
    fn equipping_apparel_displaces_only_pieces_sharing_a_slot() {
        let mut model = empty_model();
        let jumpsuit = add(&mut model, apparel("Jumpsuit", 0b0011));
        let armor = add(&mut model, apparel("Armor", 0b0100));
        let overcoat = add(&mut model, apparel("Overcoat", 0b1100));
        model.apply(PipBoyAction::Equip(jumpsuit)).unwrap();
        model.apply(PipBoyAction::Equip(armor)).unwrap();
        model.apply(PipBoyAction::Equip(overcoat)).unwrap();
        assert!(equipped(&model, jumpsuit));
        assert!(!equipped(&model, armor));
        assert!(equipped(&model, overcoat));
    }

    #[test]
    fn equipping_an_equipped_item_changes_nothing() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        model.apply(PipBoyAction::Equip(pistol)).unwrap();
        let before = model.clone();
        model.apply(PipBoyAction::Equip(pistol)).unwrap();
        assert_eq!(model, before);
    }

    #[test]
    fn equip_rejects_unknown_and_unequippable_items() {
        let mut model = empty_model();
        let stimpak = add(&mut model, aid("Stimpak", 40.0, None));
        assert_rejected(
            &mut model,
            PipBoyAction::Equip(ItemId(99)),
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
        assert_rejected(
            &mut model,
            PipBoyAction::Equip(stimpak),
            PipBoyActionError::NotEquippable(stimpak),
        );
    }

    #[test]
    fn unequip_clears_the_flag_and_rejects_what_is_not_worn() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        assert_rejected(
            &mut model,
            PipBoyAction::Unequip(pistol),
            PipBoyActionError::NotEquipped(pistol),
        );
        model.apply(PipBoyAction::Equip(pistol)).unwrap();
        model.apply(PipBoyAction::Unequip(pistol)).unwrap();
        assert!(!equipped(&model, pistol));
        assert_rejected(
            &mut model,
            PipBoyAction::Unequip(ItemId(99)),
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
    }

    #[test]
    fn using_aid_consumes_one_unit_heals_and_adds_timed_effects() {
        let mut model = empty_model();
        let mut mentats = aid("Mentats", 15.0, Some(300.0));
        mentats.count = 2;
        let id = add(&mut model, mentats);
        let effects = model.apply(PipBoyAction::Use(id)).unwrap();
        assert_eq!(model.inventory.item(id).unwrap().count, 1);
        assert!((effects.heal_hp - 15.0).abs() < f32::EPSILON);
        assert_eq!(model.character.effects.len(), 1);
        assert_eq!(model.character.effects[0].source, "Mentats");
        assert_eq!(model.character.effects[0].remaining_seconds, Some(300.0));
    }

    #[test]
    fn instant_aid_adds_no_effect_entry() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Cram", 20.0, None));
        model.apply(PipBoyAction::Use(id)).unwrap();
        assert_eq!(model.character.effects, []);
    }

    #[test]
    fn using_the_last_unit_removes_the_stack_and_frees_its_favourite_slot() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Stimpak", 40.0, None));
        model
            .apply(PipBoyAction::SetFavourite {
                item: id,
                slot: Some(FavouriteSlot(3)),
            })
            .unwrap();
        model.apply(PipBoyAction::Use(id)).unwrap();
        assert!(model.inventory.item(id).is_none());
        assert!(model.inventory.items.iter().all(|i| i.favourite.is_none()));
    }

    #[test]
    fn use_rejects_unknown_and_non_aid_items() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        assert_rejected(
            &mut model,
            PipBoyAction::Use(pistol),
            PipBoyActionError::NotUsable(pistol),
        );
        assert_rejected(
            &mut model,
            PipBoyAction::Use(ItemId(99)),
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
    }

    #[test]
    fn quick_use_finds_the_stack_by_form_key() {
        let mut model = empty_model();
        let mut other = aid("Cram", 20.0, None);
        other.form_key = Some("000001:Fallout4.esm".to_owned());
        add(&mut model, other);
        let mut stimpak = aid("Stimpak", 40.0, None);
        stimpak.form_key = Some(STIMPAK_FORM_KEY.to_owned());
        stimpak.count = 2;
        let stimpak = add(&mut model, stimpak);
        let effects = model.apply(PipBoyAction::UseFirstStimpak).unwrap();
        assert_eq!(model.inventory.item(stimpak).unwrap().count, 1);
        assert!((effects.heal_hp - 40.0).abs() < f32::EPSILON);
        assert_eq!(model.inventory.items.len(), 2);
        assert_eq!(
            model.inventory.items[0].count, 1,
            "the other stack is untouched"
        );
    }

    #[test]
    fn quick_use_radaway_finds_its_own_stack() {
        let mut model = empty_model();
        let mut stimpak = aid("Stimpak", 40.0, None);
        stimpak.form_key = Some(STIMPAK_FORM_KEY.to_owned());
        let stimpak = add(&mut model, stimpak);
        let mut radaway = aid("RadAway", 0.0, None);
        radaway.form_key = Some(RADAWAY_FORM_KEY.to_owned());
        radaway.count = 2;
        let radaway = add(&mut model, radaway);
        model.apply(PipBoyAction::UseFirstRadAway).unwrap();
        assert_eq!(model.inventory.item(radaway).unwrap().count, 1);
        assert_eq!(model.inventory.item(stimpak).unwrap().count, 1);
    }

    #[test]
    fn a_use_without_healing_returns_no_effects() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Antidote", 0.0, Some(60.0)));
        let effects = model.apply(PipBoyAction::Use(id)).unwrap();
        assert_eq!(effects, ActionEffects::default());
    }

    #[test]
    fn actions_other_than_using_aid_have_no_effects() {
        let mut model = empty_model();
        assert_eq!(
            model.apply(PipBoyAction::Open),
            Ok(ActionEffects::default())
        );
    }

    #[test]
    fn submit_returns_the_effects_of_an_accepted_action_and_none_for_a_rejected_one() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Stimpak", 25.0, None));
        assert!((model.submit(PipBoyAction::Use(id)).heal_hp - 25.0).abs() < f32::EPSILON);
        assert_eq!(
            model.submit(PipBoyAction::Use(id)),
            ActionEffects::default()
        );
        assert_eq!(
            model.last_rejection,
            Some(PipBoyActionError::UnknownItem(id))
        );
    }

    #[test]
    fn quick_use_of_a_stack_that_is_not_aid_is_not_usable() {
        let mut model = empty_model();
        let mut impostor = weapon("Stimpak-shaped club", 5.0, 1.0);
        impostor.form_key = Some(STIMPAK_FORM_KEY.to_owned());
        let impostor = add(&mut model, impostor);
        assert_rejected(
            &mut model,
            PipBoyAction::UseFirstStimpak,
            PipBoyActionError::NotUsable(impostor),
        );
    }

    #[test]
    fn quick_use_without_a_matching_stack_is_none_left() {
        let mut model = empty_model();
        assert_rejected(
            &mut model,
            PipBoyAction::UseFirstStimpak,
            PipBoyActionError::NoneLeft,
        );
        assert_rejected(
            &mut model,
            PipBoyAction::UseFirstRadAway,
            PipBoyActionError::NoneLeft,
        );
    }

    #[test]
    fn inspecting_only_records_the_request_for_a_carried_item() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        let before = model.inventory.clone();
        model.apply(PipBoyAction::RequestInspect(pistol)).unwrap();
        assert_eq!(model.inspect_request, Some(pistol));
        assert_eq!(model.inventory, before);
        assert_rejected(
            &mut model,
            PipBoyAction::RequestInspect(ItemId(99)),
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
    }

    #[test]
    fn dropping_reduces_the_count_and_removes_an_emptied_stack() {
        let mut model = empty_model();
        let mut cram = aid("Cram", 20.0, None);
        cram.count = 5;
        let id = add(&mut model, cram);
        model
            .apply(PipBoyAction::Drop { item: id, count: 2 })
            .unwrap();
        assert_eq!(model.inventory.item(id).unwrap().count, 3);
        model
            .apply(PipBoyAction::Drop { item: id, count: 3 })
            .unwrap();
        assert!(model.inventory.item(id).is_none());
    }

    #[test]
    fn dropping_the_last_unit_of_an_equipped_item_unequips_it() {
        let mut model = empty_model();
        let pistol = add(&mut model, weapon("Pistol", 10.0, 1.0));
        model.apply(PipBoyAction::Equip(pistol)).unwrap();
        model
            .apply(PipBoyAction::Drop {
                item: pistol,
                count: 1,
            })
            .unwrap();
        assert_eq!(model.inventory.items, []);
    }

    #[test]
    fn dropping_part_of_an_equipped_stack_keeps_it_equipped() {
        let mut model = empty_model();
        let mut pistol = weapon("Pistol", 10.0, 1.0);
        pistol.count = 2;
        let id = add(&mut model, pistol);
        model.apply(PipBoyAction::Equip(id)).unwrap();
        model
            .apply(PipBoyAction::Drop { item: id, count: 1 })
            .unwrap();
        assert!(equipped(&model, id));
    }

    #[test]
    fn drop_rejects_more_than_carried_and_unknown_items() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Cram", 20.0, None));
        assert_rejected(
            &mut model,
            PipBoyAction::Drop { item: id, count: 2 },
            PipBoyActionError::NotEnoughToDrop {
                requested: 2,
                available: 1,
            },
        );
        assert_rejected(
            &mut model,
            PipBoyAction::Drop {
                item: ItemId(99),
                count: 1,
            },
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
    }

    #[test]
    fn dropping_zero_units_is_rejected_and_recorded() {
        let mut model = empty_model();
        let id = add(&mut model, aid("Cram", 20.0, None));
        assert_rejected(
            &mut model,
            PipBoyAction::Drop { item: id, count: 0 },
            PipBoyActionError::ZeroQuantity,
        );
        model.submit(PipBoyAction::Drop { item: id, count: 0 });
        assert_eq!(model.last_rejection, Some(PipBoyActionError::ZeroQuantity));
    }

    #[test]
    fn apparel_without_a_body_slot_cannot_be_equipped() {
        let mut model = empty_model();
        let slotless = add(&mut model, apparel("Slotless", 0));
        assert_rejected(
            &mut model,
            PipBoyAction::Equip(slotless),
            PipBoyActionError::NotEquippable(slotless),
        );
    }

    #[test]
    fn the_custom_marker_rejects_a_non_finite_position() {
        let mut model = empty_model();
        for position in [
            glam::Vec3::new(f32::NAN, 0.0, 0.0),
            glam::Vec3::new(0.0, f32::INFINITY, 0.0),
            glam::Vec3::new(0.0, 0.0, f32::NEG_INFINITY),
        ] {
            let before = model.clone();
            let result = model.apply(PipBoyAction::PlaceCustomMarker(position));
            assert!(
                matches!(result, Err(PipBoyActionError::NonFinitePosition(_))),
                "{position:?}"
            );
            assert_eq!(model, before);
        }
    }

    #[test]
    fn assigning_a_taken_favourite_slot_moves_it_from_the_previous_owner() {
        let mut model = empty_model();
        let first = add(&mut model, weapon("First", 1.0, 1.0));
        let second = add(&mut model, weapon("Second", 1.0, 1.0));
        let slot = Some(FavouriteSlot(5));
        model
            .apply(PipBoyAction::SetFavourite { item: first, slot })
            .unwrap();
        model
            .apply(PipBoyAction::SetFavourite { item: second, slot })
            .unwrap();
        assert_eq!(model.inventory.item(first).unwrap().favourite, None);
        assert_eq!(model.inventory.item(second).unwrap().favourite, slot);
    }

    #[test]
    fn reassigning_an_item_to_another_slot_frees_the_old_one() {
        let mut model = empty_model();
        let id = add(&mut model, weapon("Gun", 1.0, 1.0));
        model
            .apply(PipBoyAction::SetFavourite {
                item: id,
                slot: Some(FavouriteSlot(0)),
            })
            .unwrap();
        model
            .apply(PipBoyAction::SetFavourite {
                item: id,
                slot: Some(FavouriteSlot(11)),
            })
            .unwrap();
        assert_eq!(
            model.inventory.item(id).unwrap().favourite,
            Some(FavouriteSlot(11))
        );
        model
            .apply(PipBoyAction::SetFavourite {
                item: id,
                slot: None,
            })
            .unwrap();
        assert_eq!(model.inventory.item(id).unwrap().favourite, None);
    }

    #[test]
    fn favourite_rejects_out_of_range_slots_and_unknown_items() {
        let mut model = empty_model();
        let id = add(&mut model, weapon("Gun", 1.0, 1.0));
        assert_rejected(
            &mut model,
            PipBoyAction::SetFavourite {
                item: id,
                slot: Some(FavouriteSlot(12)),
            },
            PipBoyActionError::InvalidFavouriteSlot(12),
        );
        assert_rejected(
            &mut model,
            PipBoyAction::SetFavourite {
                item: ItemId(99),
                slot: None,
            },
            PipBoyActionError::UnknownItem(ItemId(99)),
        );
    }

    #[test]
    fn sort_mode_is_stored_per_category_and_validated() {
        let mut model = empty_model();
        model
            .apply(PipBoyAction::SetSortMode {
                category: InventoryCategory::Weapons,
                mode: SortMode::Damage,
            })
            .unwrap();
        assert_eq!(
            model.inventory.sort[InventoryCategory::Weapons as usize],
            SortMode::Damage
        );
        assert_eq!(
            model.inventory.sort[InventoryCategory::Aid as usize],
            SortMode::Alphabetical
        );
        assert_rejected(
            &mut model,
            PipBoyAction::SetSortMode {
                category: InventoryCategory::Aid,
                mode: SortMode::Damage,
            },
            PipBoyActionError::InvalidSortMode {
                category: InventoryCategory::Aid,
                mode: SortMode::Damage,
            },
        );
    }

    #[test]
    fn component_search_tags_toggle() {
        let mut model = empty_model();
        let tag = || PipBoyAction::ToggleComponentSearchTag("Steel".to_owned());
        model.apply(tag()).unwrap();
        assert_eq!(model.inventory.tagged_components, ["Steel"]);
        model.apply(tag()).unwrap();
        assert_eq!(model.inventory.tagged_components, [] as [String; 0]);
    }

    #[test]
    fn ranking_up_spends_a_point_and_raises_the_rank() {
        let mut model = perk_model();
        model.apply(PipBoyAction::RankUpPerk(gun_nut())).unwrap();
        assert_eq!(model.perks.rank(&gun_nut()), 1);
        assert_eq!(model.character.progression.unspent_perk_points, 1);
    }

    #[test]
    fn rank_up_rejections_leave_the_model_unchanged() {
        let mut model = perk_model();
        assert_rejected(
            &mut model,
            PipBoyAction::RankUpPerk(PerkId("Nope".to_owned())),
            PipBoyActionError::UnknownPerk(PerkId("Nope".to_owned())),
        );

        model.character.progression.unspent_perk_points = 0;
        assert_rejected(
            &mut model,
            PipBoyAction::RankUpPerk(gun_nut()),
            PipBoyActionError::NoPerkPoints,
        );

        model.character.progression.unspent_perk_points = 2;
        model.character.progression.level = 4;
        assert_rejected(
            &mut model,
            PipBoyAction::RankUpPerk(gun_nut()),
            PipBoyActionError::PerkLevelTooLow {
                perk: gun_nut(),
                required_level: 5,
            },
        );

        model.character.progression.level = 10;
        model.character.special.0[Special::Intelligence as usize].base = 2;
        assert_rejected(
            &mut model,
            PipBoyAction::RankUpPerk(gun_nut()),
            PipBoyActionError::PerkSpecialTooLow { perk: gun_nut() },
        );

        model.character.special.0[Special::Intelligence as usize].base = 3;
        model.perks.0.insert(gun_nut(), 2);
        assert_rejected(
            &mut model,
            PipBoyAction::RankUpPerk(gun_nut()),
            PipBoyActionError::PerkAtMaxRank(gun_nut()),
        );
    }

    fn quest(id: &str) -> Quest {
        Quest {
            id: QuestId(id.to_owned()),
            name: id.to_owned(),
            description: String::new(),
            status: QuestStatus::Active,
            tracked: false,
            objectives: Vec::new(),
            vault_boy_swf: None,
        }
    }

    #[test]
    fn quest_tracking_toggles_one_quest() {
        let mut model = empty_model();
        model.quests = vec![quest("a"), quest("b")];
        model
            .apply(PipBoyAction::ToggleQuestTracked(QuestId("a".to_owned())))
            .unwrap();
        assert!(model.quests[0].tracked && !model.quests[1].tracked);
        model
            .apply(PipBoyAction::ToggleQuestTracked(QuestId("a".to_owned())))
            .unwrap();
        assert!(!model.quests[0].tracked);
        assert_rejected(
            &mut model,
            PipBoyAction::ToggleQuestTracked(QuestId("zzz".to_owned())),
            PipBoyActionError::UnknownQuest(QuestId("zzz".to_owned())),
        );
    }

    fn radio_model() -> PipBoyModel {
        let mut model = empty_model();
        model.radio.stations = [(1, true), (2, false)]
            .into_iter()
            .map(|(id, in_range)| RadioStation {
                id: StationId(id),
                name: format!("Station {id}"),
                frequency_mhz: 90.0 + f32::from(id),
                in_range,
            })
            .collect();
        model
    }

    #[test]
    fn tuning_needs_an_in_range_known_station() {
        let mut model = radio_model();
        model.apply(PipBoyAction::TuneRadio(StationId(1))).unwrap();
        assert_eq!(model.radio.tuned, Some(StationId(1)));
        assert_rejected(
            &mut model,
            PipBoyAction::TuneRadio(StationId(2)),
            PipBoyActionError::StationOutOfRange(StationId(2)),
        );
        assert_rejected(
            &mut model,
            PipBoyAction::TuneRadio(StationId(9)),
            PipBoyActionError::UnknownStation(StationId(9)),
        );
        model.apply(PipBoyAction::TurnOffRadio).unwrap();
        assert_eq!(model.radio.tuned, None);
    }

    #[test]
    fn the_custom_marker_is_placed_and_removed() {
        let mut model = empty_model();
        let position = glam::Vec3::new(1.0, 2.0, 3.0);
        model
            .apply(PipBoyAction::PlaceCustomMarker(position))
            .unwrap();
        assert_eq!(model.custom_marker, Some(position));
        model.apply(PipBoyAction::RemoveCustomMarker).unwrap();
        assert_eq!(model.custom_marker, None);
    }

    fn travel_model() -> PipBoyModel {
        let mut model = PipBoyModel::new(catalog_with(
            Vec::new(),
            vec![
                marker("open", true, true),
                marker("unseen", false, true),
                marker("no-travel", true, false),
            ],
        ));
        model
            .discovered_markers
            .remove(&MapMarkerId("unseen".to_owned()));
        model
    }

    #[test]
    fn fast_travel_is_only_recorded_for_discovered_travellable_markers() {
        let mut model = travel_model();
        let id = |name: &str| MapMarkerId(name.to_owned());
        model
            .apply(PipBoyAction::RequestFastTravel(id("open")))
            .unwrap();
        assert_eq!(model.fast_travel_request, Some(id("open")));
    }

    #[test]
    fn fast_travel_rejections_leave_the_model_unchanged() {
        let mut model = travel_model();
        let id = |name: &str| MapMarkerId(name.to_owned());
        for name in ["unseen", "no-travel"] {
            assert_rejected(
                &mut model,
                PipBoyAction::RequestFastTravel(id(name)),
                PipBoyActionError::CannotFastTravel(id(name)),
            );
        }
        assert_rejected(
            &mut model,
            PipBoyAction::RequestFastTravel(id("missing")),
            PipBoyActionError::UnknownMarker(id("missing")),
        );
        let mut without_map = PipBoyModel::new(std::sync::Arc::new(PipBoyCatalog::default()));
        assert_rejected(
            &mut without_map,
            PipBoyAction::RequestFastTravel(id("open")),
            PipBoyActionError::UnknownMarker(id("open")),
        );
    }

    #[test]
    fn submit_records_a_rejection_and_the_next_accepted_action_clears_it() {
        let mut model = empty_model();
        model.submit(PipBoyAction::Equip(ItemId(7)));
        assert_eq!(
            model.last_rejection,
            Some(PipBoyActionError::UnknownItem(ItemId(7)))
        );
        model.submit(PipBoyAction::Open);
        assert_eq!(model.last_rejection, None);
        assert!(model.open);
    }

    #[test]
    fn plain_items_of_other_categories_are_not_equippable() {
        let mut model = empty_model();
        let misc = add(
            &mut model,
            item("Teddy Bear", InventoryCategory::Misc, ItemDetails::Misc),
        );
        assert_rejected(
            &mut model,
            PipBoyAction::Equip(misc),
            PipBoyActionError::NotEquippable(misc),
        );
    }
}
