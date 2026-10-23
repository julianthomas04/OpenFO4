//! Pip-Boy model: the gameplay-side state the Pip-Boy screen shows and its actions mutate. Pure data
//! (glam/std only). The render layer reads it through [`PipBoySnapshot`] and changes it only by
//! submitting [`PipBoyAction`]s as `SimCommand::PipBoy`.

pub mod actions;
pub mod catalog;
pub mod character;
pub mod clock;
pub mod inventory;
pub mod map;
pub mod perks;
pub mod quests;
pub mod radio;
pub mod stats;
pub mod workshops;

#[cfg(test)]
mod test_support;

use std::collections::HashSet;
use std::sync::Arc;

use glam::Vec3;

pub use actions::{ActionEffects, PipBoyAction, PipBoyActionError};
pub use catalog::{ItemDefinition, PipBoyCatalog};
pub use character::{Character, Limb, Sex, Special};
pub use clock::GameClock;
pub use inventory::{DamageType, Inventory, InventoryCategory, InventoryItem, ItemId};
pub use map::{MapMarker, MapMarkerId, WorldMap, WorldMapProjection};
pub use perks::{OwnedPerks, PerkDefinition, PerkId};
pub use quests::{Quest, QuestId};
pub use radio::{Radio, StationId};
pub use stats::StatCategory;
pub use workshops::Settlement;

#[derive(Clone, Debug, PartialEq)]
pub struct PipBoyModel {
    pub open: bool,
    pub catalog: Arc<PipBoyCatalog>,
    pub character: Character,
    pub inventory: Inventory,
    pub perks: OwnedPerks,
    pub quests: Vec<Quest>,
    pub settlements: Vec<Settlement>,
    pub stats: Vec<StatCategory>,
    pub radio: Radio,
    pub clock: GameClock,
    pub current_location: Option<String>,
    /// Engine-space position of the player-placed map marker.
    pub custom_marker: Option<Vec3>,
    /// The map markers the player has discovered; every marker until runtime discovery exists.
    pub discovered_markers: HashSet<MapMarkerId>,
    pub fast_travel_request: Option<MapMarkerId>,
    /// The item the player asked to examine; the engine consumes it.
    pub inspect_request: Option<ItemId>,
    /// The most recent rejected action, for the screen to report; cleared by the next accepted one.
    pub last_rejection: Option<PipBoyActionError>,
}

const STARTING_CARRY_CAPACITY: f32 = 200.0;
const STARTING_XP_FOR_NEXT_LEVEL: u32 = 200;
const STARTING_CLOCK: GameClock = GameClock {
    year: 2287,
    month: 10,
    day: 23,
    minute_of_day: 9 * 60 + 47,
};

impl PipBoyModel {
    /// An empty model over `catalog`: nothing owned, nothing known.
    pub fn new(catalog: Arc<PipBoyCatalog>) -> Self {
        let mut character = Character::default();
        character.special.0 = [character::SpecialValue { base: 1, bonus: 0 }; 7];
        character.progression = character::Progression {
            level: 1,
            xp_into_level: 0,
            xp_for_next_level: STARTING_XP_FOR_NEXT_LEVEL,
            unspent_perk_points: 0,
        };
        let mut model = Self {
            open: false,
            catalog,
            character,
            inventory: Inventory {
                carry_capacity: STARTING_CARRY_CAPACITY,
                ..Inventory::default()
            },
            perks: OwnedPerks::default(),
            quests: Vec::new(),
            settlements: Vec::new(),
            stats: Vec::new(),
            radio: Radio::default(),
            clock: STARTING_CLOCK,
            current_location: None,
            custom_marker: None,
            discovered_markers: HashSet::new(),
            fast_travel_request: None,
            inspect_request: None,
            last_rejection: None,
        };
        model.discover_all_markers();
        model
    }
}

/// Live player values the Pip-Boy shows next to the model (HP/AP bars, map arrow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerVitals {
    pub hp: f32,
    pub hp_max: f32,
    pub ap: f32,
    pub ap_max: f32,
    /// Engine-space position of the soles; `None` until the player has spawned and while in an
    /// interior, where the world map has no player position.
    pub position: Option<Vec3>,
    /// Camera yaw, `PlayerInput::yaw` convention.
    pub heading_yaw: f32,
}

/// What the render layer reads each frame while the Pip-Boy is open.
#[derive(Clone, Copy, Debug)]
pub struct PipBoySnapshot<'a> {
    pub model: &'a PipBoyModel,
    pub vitals: PlayerVitals,
}

#[cfg(test)]
mod tests {
    use super::test_support::empty_model;
    use super::*;

    #[test]
    fn a_new_model_is_empty_and_closed() {
        let model = empty_model();
        assert!(!model.open);
        assert!(model.inventory.items.is_empty());
        assert!(model.quests.is_empty());
        assert!(model.settlements.is_empty());
        assert!(model.stats.is_empty());
        assert!(model.radio.stations.is_empty());
        assert!(model.perks.0.is_empty());
        assert!(model.custom_marker.is_none());
        assert!(model.last_rejection.is_none());
    }

    #[test]
    fn a_new_character_starts_at_level_one_with_base_one_everywhere() {
        let model = empty_model();
        assert_eq!(model.character.progression.level, 1);
        assert_eq!(model.character.progression.unspent_perk_points, 0);
        for special in Special::ALL {
            assert_eq!(model.character.special.get(special).base, 1);
            assert_eq!(model.character.special.get(special).bonus, 0);
        }
        assert!(
            Limb::ALL
                .iter()
                .all(|&limb| (model.character.limbs.get(limb) - 1.0).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn a_new_model_starts_on_the_authored_date() {
        let clock = empty_model().clock;
        assert_eq!((clock.year, clock.month, clock.day), (2287, 10, 23));
        assert_eq!(clock.minute_of_day, 9 * 60 + 47);
    }
}
