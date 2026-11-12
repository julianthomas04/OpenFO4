//! Text shown while aiming at a placed item: its name and whether taking it is a theft. Pure
//! rules; the callers own the data loading.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Context;

use super::door_prompt::{VerbError, non_blank, required_setting_text};
use super::rollover_prompt::{RolloverPrompt, Verb};
use super::world_streamer::PlacedItem;
use crate::assets::db::DbClient;
use crate::sim::EntityId;
use crate::sim::pipboy::{ItemDefinition, PipBoyCatalog};
use crate::startup::StartupError;
use crate::utils::warn_once;

pub(super) const TAKE_SETTING: &str = "sTake";
pub(super) const STEAL_SETTING: &str = "sSteal";
pub(super) const PLAYER_NPC: &str = "000007:Fallout4.esm";

/// The verbs of the item rollover, from the game settings.
pub(super) struct ItemPrompts {
    take: Arc<str>,
    steal: Arc<str>,
}

impl ItemPrompts {
    pub(super) fn new(take: &str, steal: &str) -> Result<Self, VerbError> {
        let verb = |text: &str, setting| {
            non_blank(Some(text))
                .map(Arc::from)
                .ok_or(VerbError::Empty { setting })
        };
        Ok(Self {
            take: verb(take, TAKE_SETTING)?,
            steal: verb(steal, STEAL_SETTING)?,
        })
    }

    /// A missing or blank verb setting stops startup.
    pub(super) fn load(db: &DbClient) -> Result<Self, StartupError> {
        Self::read(db).map_err(StartupError::ItemPromptSettings)
    }

    fn read(db: &DbClient) -> anyhow::Result<Self> {
        Ok(Self::new(
            &required_setting_text(db, TAKE_SETTING)?,
            &required_setting_text(db, STEAL_SETTING)?,
        )?)
    }
}

/// The form keys that count as "the player": the player NPC and every faction of its base
/// record.
pub(super) struct PlayerOwners {
    keys: HashSet<String>,
}

impl PlayerOwners {
    pub(super) fn new(keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }

    /// The player NPC and its factions. A database converted before the factions were extracted
    /// leaves the player NPC alone, reported.
    pub(super) fn load(db: &DbClient) -> anyhow::Result<Self> {
        let factions = db
            .get_npc_factions(PLAYER_NPC)
            .with_context(|| format!("factions of the player NPC {PLAYER_NPC}"))?
            .unwrap_or_else(|| {
                tracing::warn!(
                    "the player's factions are unknown; only items owned by {PLAYER_NPC} count as the player's"
                );
                Vec::new()
            });
        Ok(Self::new(
            std::iter::once(PLAYER_NPC.to_owned())
                .chain(factions.into_iter().map(|row| row.faction)),
        ))
    }

    pub(super) fn owns(&self, owner: &str) -> bool {
        self.keys.contains(owner)
    }
}

/// Steal = an owner exists and is not the player. Faction rank is ignored.
pub(super) fn is_stealing(owner: Option<&str>, player: &PlayerOwners) -> bool {
    owner.is_some_and(|owner| !player.owns(owner))
}

/// The item's name and, as its verb, Take or (in the warning tone) Steal. No count text.
pub(super) fn item_prompt(
    item: &PlacedItem,
    definition: &ItemDefinition,
    prompts: &ItemPrompts,
    player: &PlayerOwners,
) -> RolloverPrompt {
    let verb = if is_stealing(item.owner.as_deref(), player) {
        Verb::warning(Arc::clone(&prompts.steal))
    } else {
        Verb::normal(Arc::clone(&prompts.take))
    };
    RolloverPrompt {
        name: Arc::from(definition.name.as_str()),
        verb: Some(verb),
    }
}

/// [`item_prompt`] for an item whose base the catalog may lack: no prompt then, reported once.
pub(super) fn catalog_item_prompt(
    item: &PlacedItem,
    catalog: &PipBoyCatalog,
    prompts: &ItemPrompts,
    player: &PlayerOwners,
) -> Option<RolloverPrompt> {
    let Some(definition) = catalog.item(&item.base) else {
        warn_once(&format!("item-prompt:{}", item.base), || {
            format!(
                "placed item {:?} has base {} which the catalog lacks; it shows no prompt",
                item.entity, item.base
            )
        });
        return None;
    };
    Some(item_prompt(item, definition, prompts, player))
}

/// The prompt of the item last aimed at, so it is built once per aimed entity and not every
/// frame.
#[derive(Default)]
pub(super) struct ItemPromptCache {
    shown: Option<(EntityId, Option<RolloverPrompt>)>,
}

impl ItemPromptCache {
    /// The prompt of `entity`, from `build` unless it is the entity of the previous call.
    pub(super) fn get_or_build(
        &mut self,
        entity: EntityId,
        build: impl FnOnce() -> Option<RolloverPrompt>,
    ) -> Option<&RolloverPrompt> {
        if self.shown.as_ref().map(|(shown, _)| *shown) != Some(entity) {
            self.shown = Some((entity, build()));
        }
        self.shown.as_ref()?.1.as_ref()
    }

    /// Forgets the prompt, for when no item is aimed at.
    pub(super) fn clear(&mut self) {
        self.shown = None;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use glam::Vec3;

    use super::*;
    use crate::assets::db::test_support::database_with;
    use crate::rendering::rollover_prompt::VerbTone;
    use crate::sim::pipboy::inventory::ItemDetails;
    use crate::sim::pipboy::{InventoryCategory, ItemKind};

    const STUDIO_FACTION: &str = "0F0A01:Test.esm";
    const STRANGER: &str = "0F0B02:Test.esm";
    const WIDGET_KEY: &str = "0F3B01:Test.esm";

    fn prompts() -> ItemPrompts {
        ItemPrompts::new("Grab it", "Pinch it").unwrap()
    }

    fn player() -> PlayerOwners {
        PlayerOwners::new([PLAYER_NPC.to_owned(), STUDIO_FACTION.to_owned()])
    }

    fn item(owner: Option<&str>) -> PlacedItem {
        PlacedItem {
            entity: EntityId(9),
            base: WIDGET_KEY.to_owned(),
            count: 4,
            owner: owner.map(str::to_owned),
            position: Vec3::ZERO,
        }
    }

    fn definition() -> ItemDefinition {
        ItemDefinition {
            form_key: WIDGET_KEY.to_owned(),
            kind: ItemKind::Misc,
            editor_id: None,
            name: "Brass Widget".to_owned(),
            category: InventoryCategory::Misc,
            weight: 1.0,
            value: 7,
            details: ItemDetails::Misc,
        }
    }

    fn catalog() -> PipBoyCatalog {
        PipBoyCatalog {
            items: vec![definition()],
            ..PipBoyCatalog::default()
        }
    }

    fn verb_of(prompt: &RolloverPrompt) -> (&str, VerbTone) {
        let verb = prompt.verb.as_ref().expect("an item prompt has a verb");
        (&*verb.text, verb.tone)
    }

    #[test]
    fn an_unowned_item_is_taken_not_stolen() {
        assert!(!is_stealing(None, &player()));
    }

    #[test]
    fn an_item_of_the_player_npc_or_a_player_faction_is_taken_not_stolen() {
        assert!(!is_stealing(Some(PLAYER_NPC), &player()));
        assert!(!is_stealing(Some(STUDIO_FACTION), &player()));
    }

    #[test]
    fn an_item_of_any_other_owner_is_stolen() {
        assert!(is_stealing(Some(STRANGER), &player()));
    }

    #[test]
    fn without_player_owners_every_owned_item_is_stolen_and_an_unowned_one_is_not() {
        let nobody = PlayerOwners::new([]);
        assert!(is_stealing(Some(PLAYER_NPC), &nobody));
        assert!(!is_stealing(None, &nobody));
    }

    #[test]
    fn the_prompt_names_the_item_and_takes_what_has_no_foreign_owner() {
        for owner in [None, Some(PLAYER_NPC), Some(STUDIO_FACTION)] {
            let prompt = item_prompt(&item(owner), &definition(), &prompts(), &player());
            assert_eq!(prompt.name.as_ref(), "Brass Widget", "{owner:?}");
            assert_eq!(verb_of(&prompt), ("Grab it", VerbTone::Normal), "{owner:?}");
        }
    }

    #[test]
    fn the_prompt_of_a_foreign_owned_item_steals_in_the_warning_tone() {
        let prompt = item_prompt(&item(Some(STRANGER)), &definition(), &prompts(), &player());
        assert_eq!(prompt.name.as_ref(), "Brass Widget");
        assert_eq!(verb_of(&prompt), ("Pinch it", VerbTone::Warning));
    }

    #[test]
    fn an_item_missing_from_the_catalog_has_no_prompt() {
        let empty = PipBoyCatalog::default();
        assert_eq!(
            catalog_item_prompt(&item(None), &empty, &prompts(), &player()),
            None
        );
        let found = catalog_item_prompt(&item(None), &catalog(), &prompts(), &player());
        assert_eq!(found.unwrap().name.as_ref(), "Brass Widget");
    }

    #[test]
    fn a_blank_verb_is_refused_naming_its_setting() {
        assert_eq!(
            ItemPrompts::new("  ", "Pinch it").err(),
            Some(VerbError::Empty {
                setting: TAKE_SETTING
            })
        );
        assert_eq!(
            ItemPrompts::new("Grab it", "").err(),
            Some(VerbError::Empty {
                setting: STEAL_SETTING
            })
        );
    }

    #[test]
    fn the_cache_builds_once_per_aimed_entity_and_rebuilds_for_another() {
        let builds = Cell::new(0);
        let prompt_named = |name: &str| {
            builds.set(builds.get() + 1);
            Some(RolloverPrompt {
                name: Arc::from(name),
                verb: None,
            })
        };
        let mut cache = ItemPromptCache::default();
        let first = cache
            .get_or_build(EntityId(1), || prompt_named("one"))
            .cloned();
        let again = cache
            .get_or_build(EntityId(1), || prompt_named("never"))
            .cloned();
        assert_eq!(first, again);
        assert_eq!(builds.get(), 1);
        let second = cache.get_or_build(EntityId(2), || prompt_named("two"));
        assert_eq!(second.unwrap().name.as_ref(), "two");
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn the_cache_remembers_that_an_entity_has_no_prompt_until_cleared() {
        let builds = Cell::new(0);
        let none = || {
            builds.set(builds.get() + 1);
            None
        };
        let mut cache = ItemPromptCache::default();
        assert_eq!(cache.get_or_build(EntityId(1), none), None);
        assert_eq!(cache.get_or_build(EntityId(1), none), None);
        assert_eq!(builds.get(), 1);
        cache.clear();
        assert_eq!(cache.get_or_build(EntityId(1), none), None);
        assert_eq!(builds.get(), 2);
    }

    mod loading {
        use super::*;

        const VERBS: &str = "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
             VALUES ('1:T.esm', 'sTake', 'string', 'Grab it'), ('2:T.esm', 'sSteal', 'string', 'Pinch it');";

        #[test]
        fn both_verb_settings_present_the_prompts_load() {
            let prompts = ItemPrompts::load(&database_with(VERBS)).unwrap();
            assert_eq!(prompts.take.as_ref(), "Grab it");
            assert_eq!(prompts.steal.as_ref(), "Pinch it");
        }

        #[test]
        fn a_missing_or_blank_verb_setting_stops_startup_naming_it() {
            for (statements, named) in [
                (
                    "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                     VALUES ('1:T.esm', 'sTake', 'string', 'Grab it');",
                    STEAL_SETTING,
                ),
                (
                    "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                     VALUES ('1:T.esm', 'sTake', 'string', ' '), ('2:T.esm', 'sSteal', 'string', 'Pinch it');",
                    TAKE_SETTING,
                ),
            ] {
                let error = match ItemPrompts::load(&database_with(statements)).err() {
                    Some(error @ StartupError::ItemPromptSettings(_)) => error.to_string(),
                    other => panic!("expected the item prompt startup error, got {other:?}"),
                };
                assert!(error.contains(named), "{error}");
            }
        }

        #[test]
        fn the_player_owners_are_the_player_npc_and_its_factions_only() {
            let db = database_with(&format!(
                "INSERT INTO npc_factions (npc, faction, rank) VALUES
                    ('{PLAYER_NPC}', '{STUDIO_FACTION}', 0),
                    ('0F0C03:Test.esm', '{STRANGER}', 1);"
            ));
            let owners = PlayerOwners::load(&db).unwrap();
            assert!(owners.owns(PLAYER_NPC));
            assert!(owners.owns(STUDIO_FACTION));
            assert!(!owners.owns(STRANGER));
        }

        #[test]
        fn a_database_without_the_faction_table_leaves_the_player_npc_alone() {
            let owners = PlayerOwners::load(&database_with("DROP TABLE npc_factions;")).unwrap();
            assert!(owners.owns(PLAYER_NPC));
            assert!(!owners.owns(STUDIO_FACTION));
        }

        #[test]
        #[ignore = "needs OPENFO4_INSTALL_DIR with the reconverted database"]
        fn the_installed_database_has_the_prompt_verbs_and_the_player_faction() {
            let db = crate::assets::db::test_support::installed_database();
            let prompts = ItemPrompts::load(&db).unwrap();
            assert!(!prompts.take.trim().is_empty() && !prompts.steal.trim().is_empty());
            assert!(PlayerOwners::load(&db).unwrap().owns("01C21C:Fallout4.esm"));
        }
    }
}
