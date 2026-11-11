//! Text shown while aiming at a door: the name of the place a load door leads to and the verb of
//! the activation. A load door whose base asks for no text shows nothing at all. Pure rules; the
//! callers own the data loading.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;
use glam::Vec3;

use super::animated_objects::{AnimatedDoor, DoorState};
use super::walk_in::{PlayerLocation, WalkInTriggers};
use crate::assets::db::DbClient;
use crate::assets::db::doors::{DoorFlagsError, flags_have_no_to_text};
use crate::assets::db::map_regions::MapRegion;
use crate::assets::db::teleports::TeleportRecord;
use crate::bethesda_space::polygon_contains;
use crate::startup::StartupError;

pub(super) const OPEN_SETTING: &str = "sOpen";
pub(super) const CLOSE_SETTING: &str = "sClose";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RolloverPrompt {
    pub name: Arc<str>,
    /// `None` for a prompt that is only a name, with no key hint.
    pub verb: Option<Arc<str>>,
}

/// The text when it holds more than whitespace.
pub(super) fn non_blank(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.trim().is_empty())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum VerbError {
    #[error("game setting {setting} is empty")]
    Empty { setting: &'static str },
}

pub(super) struct PromptVerbs {
    open: Arc<str>,
    close: Arc<str>,
}

impl PromptVerbs {
    pub(super) fn new(open: &str, close: &str) -> Result<Self, VerbError> {
        let verb = |text: &str, setting| {
            non_blank(Some(text))
                .map(Arc::from)
                .ok_or(VerbError::Empty { setting })
        };
        Ok(Self {
            open: verb(open, OPEN_SETTING)?,
            close: verb(close, CLOSE_SETTING)?,
        })
    }

    /// Activating a door that is heading for open closes it.
    fn for_target(&self, target: DoorState) -> &Arc<str> {
        match target {
            DoorState::Open | DoorState::Opening => &self.close,
            DoorState::Closed | DoorState::Closing => &self.open,
        }
    }
}

/// The map regions of every worldspace, for naming a point of the open world.
#[derive(Default)]
pub(super) struct MapRegionIndex {
    by_worldspace: HashMap<String, Vec<MapRegion>>,
}

impl MapRegionIndex {
    /// Regions with a blank name never take part: they cannot name a place.
    pub(super) fn new(regions: Vec<MapRegion>) -> Self {
        let mut by_worldspace: HashMap<String, Vec<MapRegion>> = HashMap::new();
        for region in regions {
            if non_blank(Some(region.name.as_str())).is_some() {
                by_worldspace
                    .entry(region.worldspace.clone())
                    .or_default()
                    .push(region);
            }
        }
        Self { by_worldspace }
    }

    /// The map name of the highest priority region containing the point; the lower form key wins
    /// a tie.
    fn name_at(&self, worldspace: &str, point: (f32, f32)) -> Option<&str> {
        self.by_worldspace
            .get(worldspace)?
            .iter()
            .filter(|region| {
                region
                    .areas
                    .iter()
                    .any(|area| polygon_contains(area, point))
            })
            .max_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| b.form_key.cmp(&a.form_key))
            })
            .map(|region| region.name.as_str())
    }
}

/// The name of the place a load door leads to: the cell's name for an interior, the map region
/// containing the arrival point for the open world, else the worldspace's name.
fn destination_name<'a>(
    teleport: &'a TeleportRecord,
    regions: &'a MapRegionIndex,
) -> Option<&'a str> {
    if teleport.is_interior {
        return non_blank(teleport.dest_cell_name.as_deref());
    }
    let region = teleport.worldspace.as_deref().and_then(|worldspace| {
        regions.name_at(worldspace, (teleport.dest_x as f32, teleport.dest_y as f32))
    });
    region.or_else(|| non_blank(teleport.dest_worldspace_name.as_deref()))
}

/// What a load door shows, aimed at or stood near.
#[derive(Debug, PartialEq, Eq)]
enum LoadDoorText {
    /// Its base carries the no-to-text flag: it shows no text, aimed at or stood near. Walking
    /// in still starts it.
    Silent,
    /// The destination's name; `None` when it is blank.
    Destination(Option<Arc<str>>),
}

fn load_door_text(
    teleport: &TeleportRecord,
    regions: &MapRegionIndex,
) -> Result<LoadDoorText, DoorFlagsError> {
    let silent = match teleport.source_door_flags.as_deref() {
        None => false,
        flags => flags_have_no_to_text(&format!("door ref {}", teleport.source_ref), flags)?,
    };
    Ok(if silent {
        LoadDoorText::Silent
    } else {
        LoadDoorText::Destination(destination_name(teleport, regions).map(Arc::from))
    })
}

/// The labels of the load doors that show one and the silent load doors, by source ref, plus the
/// doors whose flags could not be read (they get no label and are not silent).
#[derive(Default)]
pub(super) struct LoadDoorLabels {
    pub labels: HashMap<String, Arc<str>>,
    pub silent: HashSet<String>,
    pub rejected: Vec<DoorFlagsError>,
}

pub(super) fn load_door_labels(
    teleports: &[TeleportRecord],
    regions: &MapRegionIndex,
) -> LoadDoorLabels {
    let mut result = LoadDoorLabels::default();
    for teleport in teleports {
        match load_door_text(teleport, regions) {
            Ok(LoadDoorText::Destination(Some(label))) => {
                result.labels.insert(teleport.source_ref.clone(), label);
            }
            Ok(LoadDoorText::Destination(None)) => {}
            Ok(LoadDoorText::Silent) => {
                result.silent.insert(teleport.source_ref.clone());
            }
            Err(error) => result.rejected.push(error),
        }
    }
    result
}

/// The prompt of the aimed ref. A load door wins over animation, as in `activation::resolve`:
/// a load door without a label has no prompt, it does not fall back to its animation.
pub(super) fn resolve(
    ref_id: &str,
    is_load_door: bool,
    labels: &HashMap<String, Arc<str>>,
    animated_door: Option<AnimatedDoor<'_>>,
    verbs: &PromptVerbs,
) -> Option<RolloverPrompt> {
    if is_load_door {
        return Some(RolloverPrompt {
            name: Arc::clone(labels.get(ref_id)?),
            verb: Some(Arc::clone(&verbs.open)),
        });
    }
    let door = animated_door?;
    Some(RolloverPrompt {
        name: Arc::clone(door.name?),
        verb: Some(Arc::clone(verbs.for_target(door.target))),
    })
}

/// Everything the prompt of an aimed door is made of, resolved once at startup.
pub(super) struct DoorPrompts {
    labels: HashMap<String, Arc<str>>,
    silent: HashSet<String>,
    verbs: PromptVerbs,
}

/// What the walk-in entrance that decides the HUD shows: it makes the crosshair activatable
/// whether or not it has a name. A silent entrance decides too, but shows nothing at all.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ShownEntrance {
    pub prompt: Option<RolloverPrompt>,
}

/// What the HUD shows for the aim and the entrance the player stands near.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DoorHud {
    /// The crosshair takes the activate shape.
    pub activate: bool,
    pub prompt: Option<RolloverPrompt>,
}

/// Something activatable aimed at, or an entrance near, turns the crosshair; the prompt is the
/// aimed ordinary one, else the entrance's.
pub(super) fn door_hud(
    aimed_activatable: bool,
    ordinary: Option<RolloverPrompt>,
    entrance: Option<&ShownEntrance>,
) -> DoorHud {
    DoorHud {
        activate: aimed_activatable || entrance.is_some(),
        prompt: shown_prompt(ordinary, entrance),
    }
}

/// An aimed ordinary prompt wins over the entrance the player stands near.
fn shown_prompt(
    ordinary: Option<RolloverPrompt>,
    entrance: Option<&ShownEntrance>,
) -> Option<RolloverPrompt> {
    ordinary.or_else(|| entrance.and_then(|entrance| entrance.prompt.clone()))
}

impl DoorPrompts {
    pub(super) fn new(
        labels: HashMap<String, Arc<str>>,
        silent: HashSet<String>,
        verbs: PromptVerbs,
    ) -> Self {
        Self {
            labels,
            silent,
            verbs,
        }
    }

    /// Whether the ref is a load door whose base asks for no text.
    fn is_silent(&self, ref_id: &str) -> bool {
        self.silent.contains(ref_id)
    }

    /// A missing or unusable verb setting stops startup. Unreadable map regions
    /// degrade to none, reported: exterior doors then show their worldspace's name. A database
    /// from an older installer lacks cell names and region map entries, which the schema reports
    /// once.
    pub(super) fn load(db: &DbClient, teleports: &[TeleportRecord]) -> Result<Self, StartupError> {
        let verbs = Self::load_verbs(db).map_err(StartupError::DoorPromptSettings)?;
        let regions = match db.get_map_regions() {
            Ok(regions) => MapRegionIndex::new(regions),
            Err(error) => {
                tracing::warn!("map regions not loaded ({error:#}); no door shows a map region");
                MapRegionIndex::default()
            }
        };
        let LoadDoorLabels {
            labels,
            silent,
            rejected,
        } = load_door_labels(teleports, &regions);
        if !rejected.is_empty() {
            let refs: Vec<String> = rejected.iter().map(ToString::to_string).collect();
            tracing::warn!(
                "{} load doors show no name: {}",
                rejected.len(),
                refs.join("; ")
            );
        }
        tracing::info!(
            "door prompts: {} of {} load doors have a name, {} show no text",
            labels.len(),
            teleports.len(),
            silent.len()
        );
        Ok(Self::new(labels, silent, verbs))
    }

    fn load_verbs(db: &DbClient) -> anyhow::Result<PromptVerbs> {
        let text = |name: &'static str| {
            db.get_game_setting_string(name)?
                .with_context(|| format!("game setting {name} is missing from the database"))
        };
        Ok(PromptVerbs::new(
            &text(OPEN_SETTING)?,
            &text(CLOSE_SETTING)?,
        )?)
    }

    /// What the entrance the walk-in rules pick at `feet` shows: nothing when it is silent, else
    /// its name only, no verb.
    pub(super) fn entrance(
        &self,
        triggers: &WalkInTriggers,
        location: PlayerLocation<'_>,
        feet: Vec3,
    ) -> Option<ShownEntrance> {
        let source_ref = triggers.nearest_entrance(location, feet)?;
        if self.is_silent(source_ref) {
            return None;
        }
        let prompt = self.labels.get(source_ref).map(|name| RolloverPrompt {
            name: Arc::clone(name),
            verb: None,
        });
        Some(ShownEntrance { prompt })
    }

    /// The prompt of the ref under the crosshair, if any; `is_load_door` and `door` answer for
    /// that ref.
    pub(super) fn prompt_for<'a>(
        &self,
        aimed_ref: Option<&str>,
        is_load_door: impl FnOnce(&str) -> bool,
        door: impl FnOnce(&str) -> Option<AnimatedDoor<'a>>,
    ) -> Option<RolloverPrompt> {
        let ref_id = aimed_ref?;
        resolve(
            ref_id,
            is_load_door(ref_id),
            &self.labels,
            door(ref_id),
            &self.verbs,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bethesda_space::position_to_engine;
    use crate::rendering::walk_in::{EntranceDistances, TEST_DISTANCES};

    const WORLD: &str = "WS";
    const SQUARE: [(f32, f32); 4] = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

    fn region(form_key: &str, worldspace: &str, name: &str, priority: i64) -> MapRegion {
        MapRegion {
            form_key: form_key.to_string(),
            worldspace: worldspace.to_string(),
            name: name.to_string(),
            priority,
            areas: vec![SQUARE.to_vec()],
        }
    }

    fn exterior(x: f64, y: f64) -> TeleportRecord {
        TeleportRecord {
            source_ref: "door".to_string(),
            dest_x: x,
            dest_y: y,
            dest_z: 0.0,
            dest_rot_z: 0.0,
            is_interior: false,
            worldspace: Some(WORLD.to_string()),
            dest_cell: "persistent".to_string(),
            flags: 0,
            dest_cell_name: None,
            dest_worldspace_name: Some("Commonwealth".to_string()),
            source_door_flags: Some("0".to_string()),
        }
    }

    fn interior(cell_name: Option<&str>) -> TeleportRecord {
        TeleportRecord {
            is_interior: true,
            worldspace: None,
            dest_cell_name: cell_name.map(str::to_string),
            dest_worldspace_name: None,
            ..exterior(0.0, 0.0)
        }
    }

    fn with_base_flags(mut teleport: TeleportRecord, flags: &str) -> TeleportRecord {
        teleport.source_door_flags = Some(flags.to_string());
        teleport
    }

    fn text(teleport: &TeleportRecord, regions: Vec<MapRegion>) -> LoadDoorText {
        load_door_text(teleport, &MapRegionIndex::new(regions)).unwrap()
    }

    fn label(teleport: &TeleportRecord, regions: Vec<MapRegion>) -> Option<String> {
        match text(teleport, regions) {
            LoadDoorText::Destination(label) => label.map(|label| label.to_string()),
            LoadDoorText::Silent => panic!("{} is silent", teleport.source_ref),
        }
    }

    fn verbs() -> PromptVerbs {
        PromptVerbs::new("Open", "Close").unwrap()
    }

    fn labels_of(entries: &[(&str, &str)]) -> HashMap<String, Arc<str>> {
        entries
            .iter()
            .map(|(source, text)| ((*source).to_string(), Arc::from(*text)))
            .collect()
    }

    fn door(name: Option<&Arc<str>>, target: DoorState) -> AnimatedDoor<'_> {
        AnimatedDoor { name, target }
    }

    #[test]
    fn a_point_inside_a_region_takes_its_map_name() {
        let regions = vec![region("R1", WORLD, "Back Bay", 1)];
        assert_eq!(
            label(&exterior(50.0, 50.0), regions),
            Some("Back Bay".to_string())
        );
    }

    #[test]
    fn the_higher_priority_wins_among_containing_regions() {
        let regions = vec![
            region("R1", WORLD, "Low", 1),
            region("R2", WORLD, "High", 9),
            region("R3", WORLD, "Middle", 5),
        ];
        assert_eq!(
            label(&exterior(50.0, 50.0), regions),
            Some("High".to_string())
        );
    }

    #[test]
    fn equal_priority_goes_to_the_lower_form_key() {
        let regions = vec![
            region("0788F8:Fallout4.esm", WORLD, "Later", 4),
            region("0788F7:Fallout4.esm", WORLD, "Earlier", 4),
        ];
        assert_eq!(
            label(&exterior(50.0, 50.0), regions),
            Some("Earlier".to_string())
        );
    }

    #[test]
    fn a_region_of_another_worldspace_is_ignored() {
        let regions = vec![region("R1", "OTHER", "Elsewhere", 9)];
        assert_eq!(
            label(&exterior(50.0, 50.0), regions),
            Some("Commonwealth".to_string())
        );
    }

    #[test]
    fn a_point_outside_every_region_gives_the_worldspace_name() {
        let regions = vec![region("R1", WORLD, "Back Bay", 1)];
        assert_eq!(
            label(&exterior(500.0, 50.0), regions),
            Some("Commonwealth".to_string())
        );
    }

    #[test]
    fn a_point_on_a_region_edge_counts_as_inside() {
        let regions = vec![region("R1", WORLD, "Back Bay", 1)];
        assert_eq!(
            label(&exterior(100.0, 50.0), regions),
            Some("Back Bay".to_string())
        );
    }

    #[test]
    fn a_region_without_polygons_never_matches() {
        let mut empty = region("R1", WORLD, "Nowhere", 9);
        empty.areas.clear();
        assert_eq!(
            label(&exterior(50.0, 50.0), vec![empty]),
            Some("Commonwealth".to_string())
        );
    }

    #[test]
    fn a_region_with_a_blank_name_does_not_compete() {
        let regions = vec![
            region("R1", WORLD, "  ", 9),
            region("R2", WORLD, "Back Bay", 1),
        ];
        assert_eq!(
            label(&exterior(50.0, 50.0), regions),
            Some("Back Bay".to_string())
        );
    }

    #[test]
    fn an_interior_uses_the_cell_name_and_ignores_regions() {
        let regions = vec![region("R1", WORLD, "Back Bay", 9)];
        assert_eq!(
            label(&interior(Some("Museum of Freedom")), regions),
            Some("Museum of Freedom".to_string())
        );
    }

    #[test]
    fn an_exterior_without_any_name_has_no_label() {
        let mut teleport = exterior(500.0, 50.0);
        teleport.dest_worldspace_name = None;
        assert_eq!(label(&teleport, Vec::new()), None);
    }

    #[test]
    fn whitespace_only_names_count_as_empty() {
        assert_eq!(label(&interior(Some("   ")), Vec::new()), None);
        let mut teleport = exterior(500.0, 50.0);
        teleport.dest_worldspace_name = Some(" \t".to_string());
        assert_eq!(label(&teleport, Vec::new()), None);
    }

    #[test]
    fn a_load_door_whose_base_has_no_to_text_is_silent_whatever_its_destination() {
        for flags in ["64", "66", "72"] {
            let named = with_base_flags(interior(Some("Museum of Freedom")), flags);
            assert_eq!(text(&named, Vec::new()), LoadDoorText::Silent, "{flags}");
            let regions = vec![region("R1", WORLD, "Back Bay", 1)];
            let exterior = with_base_flags(exterior(50.0, 50.0), flags);
            assert_eq!(text(&exterior, regions), LoadDoorText::Silent, "{flags}");
        }
    }

    #[test]
    fn a_load_door_whose_base_lacks_no_to_text_shows_its_destination() {
        for flags in ["0", "2", "191"] {
            let teleport = with_base_flags(interior(Some("Museum of Freedom")), flags);
            assert_eq!(
                label(&teleport, Vec::new()),
                Some("Museum of Freedom".to_string()),
                "{flags}"
            );
        }
    }

    #[test]
    fn a_load_door_without_a_base_record_shows_its_destination() {
        let mut teleport = interior(Some("Museum of Freedom"));
        teleport.source_door_flags = None;
        assert_eq!(
            label(&teleport, Vec::new()),
            Some("Museum of Freedom".to_string())
        );
    }

    #[test]
    fn malformed_flags_reject_that_door_only_and_name_it() {
        let mut bad = interior(Some("Bad"));
        bad.source_ref = "bad_door".to_string();
        bad.source_door_flags = Some("sixty four".to_string());
        let mut good = interior(Some("Good"));
        good.source_ref = "good_door".to_string();

        let result = load_door_labels(&[bad, good], &MapRegionIndex::default());

        assert_eq!(result.labels.len(), 1);
        assert_eq!(result.labels["good_door"].as_ref(), "Good");
        assert_eq!(result.rejected.len(), 1);
        assert!(
            result.rejected[0].to_string().contains("bad_door"),
            "{}",
            result.rejected[0]
        );
    }

    #[test]
    fn only_labelled_doors_enter_the_label_map_and_only_flagged_ones_are_silent() {
        let mut nameless = interior(None);
        nameless.source_ref = "nameless".to_string();
        let mut named = interior(Some("Cell"));
        named.source_ref = "named".to_string();
        let mut silent = with_base_flags(interior(Some("Cell")), "66");
        silent.source_ref = "silent".to_string();

        let result = load_door_labels(&[nameless, named, silent], &MapRegionIndex::default());

        assert_eq!(result.labels.keys().collect::<Vec<_>>(), ["named"]);
        assert_eq!(result.silent, HashSet::from(["silent".to_string()]));
        assert!(result.rejected.is_empty());
    }

    #[test]
    fn an_open_or_opening_ordinary_door_says_close() {
        let name: Arc<str> = Arc::from("Door");
        for target in [DoorState::Open, DoorState::Opening] {
            let prompt = resolve(
                "d",
                false,
                &HashMap::new(),
                Some(door(Some(&name), target)),
                &verbs(),
            )
            .unwrap();
            assert_eq!(prompt.verb.as_deref(), Some("Close"), "{target:?}");
        }
    }

    #[test]
    fn a_closed_or_closing_ordinary_door_says_open() {
        let name: Arc<str> = Arc::from("Door");
        for target in [DoorState::Closed, DoorState::Closing] {
            let prompt = resolve(
                "d",
                false,
                &HashMap::new(),
                Some(door(Some(&name), target)),
                &verbs(),
            )
            .unwrap();
            assert_eq!(
                prompt,
                RolloverPrompt {
                    name: Arc::clone(&name),
                    verb: Some(Arc::from("Open"))
                },
                "{target:?}"
            );
        }
    }

    #[test]
    fn an_ordinary_door_without_a_name_has_no_prompt() {
        assert_eq!(
            resolve(
                "d",
                false,
                &HashMap::new(),
                Some(door(None, DoorState::Closed)),
                &verbs()
            ),
            None
        );
    }

    #[test]
    fn a_load_door_wins_over_animation_and_always_says_open() {
        let animated_name: Arc<str> = Arc::from("Door");
        let labels = labels_of(&[("d", "Museum of Freedom")]);
        let prompt = resolve(
            "d",
            true,
            &labels,
            Some(door(Some(&animated_name), DoorState::Open)),
            &verbs(),
        )
        .unwrap();
        assert_eq!(prompt.name.as_ref(), "Museum of Freedom");
        assert_eq!(prompt.verb.as_deref(), Some("Open"));
    }

    #[test]
    fn a_load_door_without_a_label_does_not_fall_back_to_its_animation() {
        let animated_name: Arc<str> = Arc::from("Door");
        assert_eq!(
            resolve(
                "d",
                true,
                &HashMap::new(),
                Some(door(Some(&animated_name), DoorState::Closed)),
                &verbs()
            ),
            None
        );
    }

    #[test]
    fn a_ref_that_is_neither_a_load_door_nor_an_animated_door_has_no_prompt() {
        let labels = labels_of(&[("other", "Elsewhere")]);
        assert_eq!(resolve("d", false, &labels, None, &verbs()), None);
    }

    mod real_install {
        use std::collections::BTreeMap;

        use super::*;
        use crate::assets::db::DbClient;
        use crate::assets::db::teleports::{DoorLocation, WalkInDoor};
        use crate::assets::db::test_support::installed_database as database;

        const ORIGINAL_LABELS: [(&str, &str); 19] = [
            ("0788F7", "COMMONWEALTH"),
            ("160B23", "COMMONWEALTH"),
            ("0A3701", "COMMONWEALTH"),
            ("1DE805", "COMMONWEALTH"),
            ("15B5BB", "COMMONWEALTH"),
            ("01AC07", "MUSEUM OF FREEDOM"),
            ("07895C", "CONCORD SPEAKEASY"),
            ("02C9A5", "CONCORD CIVIC ACCESS"),
            ("0A9709", "CAMBRIDGE"),
            ("024F63", "FINANCIAL DISTRICT"),
            ("022690", "GOODNEIGHBOR"),
            ("0226B9", "GOODNEIGHBOR"),
            ("001F63", "DIAMOND CITY"),
            ("066715", "BACK BAY"),
            ("0A8C64", "NORTH END"),
            ("020285", "THEATER DISTRICT"),
            ("02BEAC", "CHARLESTOWN"),
            ("0B250F", ""),
            ("0F99FB", ""),
        ];

        #[test]
        #[ignore = "needs an installation converted with cell names and region map entries"]
        fn the_label_rule_reproduces_the_original_game() {
            let db = database();
            let teleports = db.get_placed_teleports().unwrap();
            let regions = MapRegionIndex::new(db.get_map_regions().unwrap());
            let result = load_door_labels(&teleports, &regions);
            assert!(result.rejected.is_empty(), "{:?}", result.rejected);

            for (source, expected) in ORIGINAL_LABELS {
                let source_ref = format!("{source}:Fallout4.esm");
                let shown = result
                    .labels
                    .get(&source_ref)
                    .map(|label| label.to_uppercase());
                let expected = (!expected.is_empty()).then(|| expected.to_string());
                assert_eq!(
                    result.silent.contains(&source_ref),
                    expected.is_none(),
                    "{source_ref}"
                );
                assert_eq!(shown, expected, "{source_ref}");
            }
        }

        /// `AutoloadDoorNoText`: the Vault 111 elevator.
        const NO_TEXT_ENTRANCES: [&str; 4] = [
            "01F180:Fallout4.esm",
            "0998E3:Fallout4.esm",
            "0CA2AF:Fallout4.esm",
            "0CA858:Fallout4.esm",
        ];

        #[test]
        #[ignore = "needs an installation converted with cell names and region map entries"]
        fn the_no_text_entrances_have_no_label_and_are_silent() {
            let db = database();
            let teleports = db.get_placed_teleports().unwrap();
            let regions = MapRegionIndex::new(db.get_map_regions().unwrap());
            let result = load_door_labels(&teleports, &regions);
            let walk_ins = db.get_walk_in_doors().unwrap().doors;

            for source_ref in NO_TEXT_ENTRANCES {
                assert!(
                    walk_ins.iter().any(|door| door.source_ref == source_ref),
                    "{source_ref} is not a walk-in entrance"
                );
                assert_eq!(result.labels.get(source_ref), None, "{source_ref}");
                assert!(result.silent.contains(source_ref), "{source_ref}");
            }
        }

        /// A pose of the original game: feet from `getpos`, Bethesda units.
        struct Observed {
            label: &'static str,
            door: &'static str,
            feet: [f32; 3],
            shown: Option<&'static str>,
        }

        const fn observed(
            label: &'static str,
            door: &'static str,
            feet: [f32; 3],
            shown: Option<&'static str>,
        ) -> Observed {
            Observed {
                label,
                door,
                feet,
                shown,
            }
        }

        const DIAMOND_CITY: Option<&str> = Some("DIAMOND CITY");
        const VIRGIL: Option<&str> = Some("VIRGIL'S LABORATORY");
        const MOLE_RAT_DEN: Option<&str> = Some("MOLE RAT DEN");

        /// From `walkins_openfo4_compare.json`. At `C_W2p_gate` the aimed gate's own prompt
        /// covered the entrance's name, which the rule still selects there.
        const OBSERVED: [Observed; 17] = [
            observed("C_W2_d560", "12F822", [-14540.22, -28877.11, 1463.34], None),
            observed("C_W2_d540", "12F822", [-14530.79, -28858.12, 1463.34], None),
            observed(
                "C_W2_d500",
                "12F822",
                [-14513.11, -28818.99, 1467.88],
                DIAMOND_CITY,
            ),
            observed(
                "C_W2_d450_turned180",
                "12F822",
                [-14490.12, -28769.68, 1471.13],
                DIAMOND_CITY,
            ),
            observed(
                "C_W2_d330",
                "12F822",
                [-14434.32, -28664.02, 1502.05],
                DIAMOND_CITY,
            ),
            observed(
                "C_W2p_gate",
                "12F822",
                [-14513.11, -28818.99, 1467.88],
                DIAMOND_CITY,
            ),
            observed(
                "C_W2s_start_pose",
                "12F822",
                [-14433.26, -28666.57, 1499.93],
                DIAMOND_CITY,
            ),
            observed(
                "C_W2s_walk1_last_no_start",
                "12F822",
                [-14429.44, -28657.96, 1506.44],
                DIAMOND_CITY,
            ),
            observed(
                "C_W3_last_none",
                "15A1CB",
                [-116_454.96, -136_942.34, 9944.76],
                None,
            ),
            observed(
                "C_W3_first_prompt",
                "15A1CB",
                [-116_463.77, -136_941.06, 9942.02],
                VIRGIL,
            ),
            observed(
                "C_W3_last_no_start",
                "15A1CB",
                [-116_575.52, -137_102.52, 9933.68],
                VIRGIL,
            ),
            observed(
                "C_W1_last_none",
                "08EB65",
                [-69779.81, 77141.68, 6997.22],
                None,
            ),
            observed(
                "C_W1_A552_none",
                "08EB65",
                [-69775.13, 77160.03, 6992.6],
                None,
            ),
            observed(
                "C_W1_A545_none",
                "08EB65",
                [-69771.81, 77166.43, 6990.58],
                None,
            ),
            observed(
                "C_W1_A356_none",
                "08EB65",
                [-69725.05, 77356.13, 6976.94],
                None,
            ),
            observed(
                "C_W1_A344_none",
                "08EB65",
                [-69721.7, 77369.2, 6975.71],
                None,
            ),
            observed(
                "C_W1_B141_prompt",
                "08EB65",
                [-69675.53, 77546.68, 6912.9],
                MOLE_RAT_DEN,
            ),
        ];

        /// From `display_boundary_probes.json`: per site and line, the last pose that showed the
        /// name and the first that did not.
        const DISPLAY_BOUNDARY: [Observed; 34] = [
            observed(
                "Z_W1_up_last_shown",
                "08EB65",
                [-69646.37, 77664.3, 7187.5],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_up_first_none",
                "08EB65",
                [-69646.37, 77664.3, 7188.76],
                None,
            ),
            observed(
                "Z_W1_clear_air_last_shown",
                "08EB65",
                [-69710.62, 77412.7, 7011.78],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_clear_air_first_none",
                "08EB65",
                [-69710.98, 77411.29, 7012.36],
                None,
            ),
            observed(
                "Z_W1_down_last_shown",
                "08EB65",
                [-69646.37, 77664.3, 6628.31],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_down_first_none",
                "08EB65",
                [-69646.37, 77664.3, 6627.05],
                None,
            ),
            observed(
                "Z_W1_plus_y_last_shown",
                "08EB65",
                [-69715.54, 77393.41, 6907.9],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_plus_y_first_none",
                "08EB65",
                [-69715.85, 77392.19, 6907.9],
                None,
            ),
            observed(
                "Z_W1_plus_x_last_shown",
                "08EB65",
                [-69917.27, 77733.48, 6907.9],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_plus_x_first_none",
                "08EB65",
                [-69918.48, 77733.79, 6907.9],
                None,
            ),
            observed(
                "Z_W1_yz_up45_last_shown",
                "08EB65",
                [-69695.28, 77472.75, 7105.61],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_yz_up45_first_none",
                "08EB65",
                [-69695.5, 77471.88, 7106.5],
                None,
            ),
            observed(
                "Z_W1_yz_down45_last_shown",
                "08EB65",
                [-69695.28, 77472.75, 6710.2],
                MOLE_RAT_DEN,
            ),
            observed(
                "Z_W1_yz_down45_first_none",
                "08EB65",
                [-69696.16, 77469.3, 6706.64],
                None,
            ),
            observed(
                "Z_W2_up_last_shown",
                "12F822",
                [-14317.12, -28398.68, 2160.64],
                DIAMOND_CITY,
            ),
            observed(
                "Z_W2_up_first_none",
                "12F822",
                [-14317.12, -28398.68, 2162.74],
                None,
            ),
            observed(
                "Z_W2_down_last_shown",
                "12F822",
                [-14317.12, -28398.68, 1140.05],
                DIAMOND_CITY,
            ),
            observed(
                "Z_W2_down_first_none",
                "12F822",
                [-14317.12, -28398.68, 1136.54],
                None,
            ),
            observed(
                "Z_W2_plus_y_last_shown",
                "12F822",
                [-14533.2, -28862.07, 1650.06],
                DIAMOND_CITY,
            ),
            observed(
                "Z_W2_plus_y_first_none",
                "12F822",
                [-14534.76, -28865.42, 1650.06],
                None,
            ),
            observed(
                "Z_W2_yz_up45_last_shown",
                "12F822",
                [-14469.67, -28725.82, 2011.02],
                DIAMOND_CITY,
            ),
            observed(
                "Z_W2_yz_up45_first_none",
                "12F822",
                [-14470.28, -28727.13, 2012.46],
                None,
            ),
            observed(
                "Z_W3_up_last_shown",
                "15A1CB",
                [-116_853.8, -137_324.72, 10474.36],
                VIRGIL,
            ),
            observed(
                "Z_W3_up_first_none",
                "15A1CB",
                [-116_853.8, -137_324.72, 10476.39],
                None,
            ),
            observed(
                "Z_W3_down_last_shown",
                "15A1CB",
                [-116_853.8, -137_324.72, 9377.27],
                VIRGIL,
            ),
            observed(
                "Z_W3_down_first_none",
                "15A1CB",
                [-116_853.8, -137_324.72, 9374.64],
                None,
            ),
            observed(
                "Z_W3_yz_up45_last_shown",
                "15A1CB",
                [-116_565.62, -137_065.67, 10313.13],
                VIRGIL,
            ),
            observed(
                "Z_W3_yz_up45_first_none",
                "15A1CB",
                [-116_564.44, -137_064.61, 10314.73],
                None,
            ),
            observed(
                "Z_W3_plus_x_last_shown",
                "15A1CB",
                [-116_486.74, -137_733.05, 9925.64],
                VIRGIL,
            ),
            observed(
                "Z_W3_plus_x_first_none",
                "15A1CB",
                [-116_484.09, -137_736.0, 9925.64],
                None,
            ),
            observed(
                "Z_W3_minus_y_last_shown",
                "15A1CB",
                [-117_261.98, -137_691.64, 9925.64],
                VIRGIL,
            ),
            observed(
                "Z_W3_minus_y_first_none",
                "15A1CB",
                [-117_264.9, -137_694.27, 9925.64],
                None,
            ),
            observed(
                "Z_W3_xy_diag_last_shown",
                "15A1CB",
                [-116_307.31, -137_353.81, 9925.64],
                VIRGIL,
            ),
            observed(
                "Z_W3_xy_diag_first_none",
                "15A1CB",
                [-116_303.72, -137_354.0, 9925.64],
                None,
            ),
        ];

        const NO_START: [&str; 6] = [
            "C_W2_d330",
            "C_W2s_start_pose",
            "C_W2s_walk1_last_no_start",
            "C_W3_last_no_start",
            "C_W1_A344_none",
            "C_W1_B141_prompt",
        ];

        /// Feet where the original started the Vault 111 elevator.
        const VAULT_ELEVATOR: &str = "01F180:Fallout4.esm";
        const VAULT_ELEVATOR_START: [f32; 3] = [-5461.1, -266.6, 1460.2];

        fn door_of<'a>(doors: &'a [WalkInDoor], source_ref: &str) -> &'a WalkInDoor {
            doors
                .iter()
                .find(|door| door.source_ref == source_ref)
                .unwrap_or_else(|| panic!("{source_ref} is not a walk-in entrance"))
        }

        fn location_of<'a>(doors: &'a [WalkInDoor], source_ref: &str) -> PlayerLocation<'a> {
            match &door_of(doors, source_ref).location {
                DoorLocation::Interior { cell } => PlayerLocation::Interior(cell),
                DoorLocation::Exterior { worldspace } => PlayerLocation::Exterior(worldspace),
            }
        }

        fn engine_feet([x, y, z]: [f32; 3]) -> Vec3 {
            position_to_engine(x, y, z)
        }

        /// What starts when the player walks straight from far out to `feet`, on the line from the
        /// door's origin through `feet`, so the path comes no closer to the origin than `feet`.
        fn started_on_arrival(
            triggers: &mut WalkInTriggers,
            doors: &[WalkInDoor],
            source_ref: &str,
            feet: [f32; 3],
        ) -> Option<String> {
            let location = location_of(doors, source_ref);
            let origin = Vec3::from(door_of(doors, source_ref).position.map(|v| v as f32));
            let feet = Vec3::from(feet);
            let far_out = origin + (feet - origin) * 100.0;
            triggers.rearm();
            triggers.sample(location, Some(engine_feet(far_out.to_array())));
            triggers
                .sample(location, Some(engine_feet(feet.to_array())))
                .map(str::to_owned)
        }

        fn walk_ins(db: &DbClient) -> (Vec<WalkInDoor>, WalkInTriggers) {
            let doors = db.get_walk_in_doors().unwrap().doors;
            let distances = EntranceDistances::load(db).unwrap();
            let (triggers, report) = WalkInTriggers::build(&doors, distances);
            assert!(report.non_finite.is_empty(), "{report:?}");
            (doors, triggers)
        }

        #[test]
        #[ignore = "needs an installation converted with cell names and region map entries"]
        fn entrances_show_their_names_where_the_original_did() {
            let db = database();
            let teleports = db.get_placed_teleports().unwrap();
            let prompts = DoorPrompts::load(&db, &teleports).unwrap();
            let (doors, triggers) = walk_ins(&db);
            for pose in OBSERVED.iter().chain(&DISPLAY_BOUNDARY) {
                let location = location_of(&doors, &format!("{}:Fallout4.esm", pose.door));
                let shown = prompts
                    .entrance(&triggers, location, engine_feet(pose.feet))
                    .map(|entrance| entrance.prompt.map(|prompt| prompt.name.to_uppercase()));
                let expected = pose.shown.map(|name| Some(name.to_string()));
                assert_eq!(shown, expected, "{}", pose.label);
            }
        }

        #[test]
        #[ignore = "needs an installation converted with cell names and region map entries"]
        fn entrances_start_where_the_original_did() {
            let db = database();
            let (doors, mut triggers) = walk_ins(&db);

            for label in NO_START {
                let pose = OBSERVED.iter().find(|pose| pose.label == label).unwrap();
                let source_ref = format!("{}:Fallout4.esm", pose.door);
                assert_eq!(
                    started_on_arrival(&mut triggers, &doors, &source_ref, pose.feet),
                    None,
                    "{label}"
                );
            }
            assert_eq!(
                started_on_arrival(&mut triggers, &doors, VAULT_ELEVATOR, VAULT_ELEVATOR_START),
                Some(VAULT_ELEVATOR.to_string())
            );
        }

        #[test]
        #[ignore = "needs an installation converted with cell names and region map entries"]
        fn exterior_destination_names_split_into_four_places_and_fifteen_map_regions() {
            let db = database();
            let teleports = db.get_placed_teleports().unwrap();
            let regions = MapRegionIndex::new(db.get_map_regions().unwrap());

            let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
            for teleport in teleports.iter().filter(|teleport| !teleport.is_interior) {
                let name = destination_name(teleport, &regions).expect("an exterior name");
                *by_name.entry(name).or_default() += 1;
            }

            let places = [
                ("Commonwealth", 158),
                ("Diamond City", 47),
                ("Goodneighbor", 12),
                ("Sanctuary Hills", 1),
            ];
            for (name, count) in places {
                assert_eq!(by_name.remove(name), Some(count), "{name}");
            }
            assert_eq!(by_name.len(), 15, "{by_name:?}");
            assert_eq!(by_name.values().sum::<usize>(), 218);
        }
    }

    mod loading {
        use super::*;
        use crate::assets::db::test_support::database_with;

        const VERBS: &str = "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
             VALUES ('1:T.esm', 'sOpen', 'string', 'Open'), ('2:T.esm', 'sClose', 'string', 'Close');";

        const WORLD: &str = "INSERT INTO worldspaces (form_key, name) VALUES ('WS', 'Commonwealth');
             INSERT INTO cells (form_key, is_interior) VALUES ('INT', 1);
             INSERT INTO cells (form_key, worldspace, grid_x, grid_y) VALUES ('EXT', 'WS', 0, 0);
             INSERT INTO placed_objects (form_key, cell_form_key, base_object, object_type,
                 position_x, position_y, position_z, rotation_x, rotation_y, rotation_z)
             VALUES ('dest_int', 'INT', 'b', 'PlacedObject', 0, 0, 0, 0, 0, 0),
                    ('dest_ext', 'EXT', 'b', 'PlacedObject', 0, 0, 0, 0, 0, 0);
             INSERT INTO placed_teleports (source_ref, destination_ref, dest_position_x,
                 dest_position_y, dest_position_z, dest_rotation_x, dest_rotation_y, dest_rotation_z)
             VALUES ('door_in', 'dest_int', 1, 2, 3, 0, 0, 0),
                    ('door_out', 'dest_ext', 1, 2, 3, 0, 0, 0);";

        fn load(statements: &str) -> Result<DoorPrompts, StartupError> {
            let db = database_with(statements);
            let teleports = db.get_placed_teleports().unwrap();
            DoorPrompts::load(&db, &teleports)
        }

        fn verb_failure(statements: &str) -> String {
            match load(statements).err() {
                Some(error @ StartupError::DoorPromptSettings(_)) => error.to_string(),
                other => panic!("expected the verb startup error, got {other:?}"),
            }
        }

        fn shown(prompts: &DoorPrompts, source_ref: &str) -> Option<(String, String)> {
            prompts
                .prompt_for(Some(source_ref), |_| true, |_| None)
                .map(|prompt| (prompt.name.to_string(), prompt.verb.unwrap().to_string()))
        }

        #[test]
        fn both_verbs_present_the_doors_are_labelled_and_say_open() {
            let prompts = load(&format!(
                "{VERBS}{WORLD} UPDATE cells SET name = 'Museum of Freedom' WHERE form_key = 'INT';"
            ))
            .unwrap();
            assert_eq!(
                shown(&prompts, "door_in"),
                Some(("Museum of Freedom".to_string(), "Open".to_string()))
            );
            assert_eq!(
                shown(&prompts, "door_out"),
                Some(("Commonwealth".to_string(), "Open".to_string()))
            );
        }

        #[test]
        fn a_load_door_of_a_named_base_with_no_to_text_shows_no_text() {
            let prompts = load(&format!(
                "{VERBS}{WORLD}
                 UPDATE cells SET name = 'Museum of Freedom' WHERE form_key = 'INT';
                 INSERT INTO doors (form_key, name, flags) VALUES ('auto', 'Auto Door', '66');
                 INSERT INTO placed_objects (form_key, cell_form_key, base_object, object_type,
                     position_x, position_y, position_z, rotation_x, rotation_y, rotation_z)
                 VALUES ('door_in', 'EXT', 'auto', 'PlacedObject', 0, 0, 0, 0, 0, 0);"
            ))
            .unwrap();
            assert_eq!(shown(&prompts, "door_in"), None);
            assert_eq!(
                shown(&prompts, "door_out"),
                Some(("Commonwealth".to_string(), "Open".to_string()))
            );
        }

        #[test]
        fn a_database_from_an_older_installer_loads_with_worldspace_names_only() {
            let prompts = load(&format!(
                "{VERBS}{WORLD}
                 ALTER TABLE cells DROP COLUMN name;
                 ALTER TABLE regions DROP COLUMN map_priority;"
            ))
            .unwrap();
            assert_eq!(shown(&prompts, "door_in"), None);
            assert_eq!(
                shown(&prompts, "door_out"),
                Some(("Commonwealth".to_string(), "Open".to_string()))
            );
        }

        #[test]
        fn a_missing_open_setting_stops_startup_naming_it() {
            let message = verb_failure(
                "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                 VALUES ('2:T.esm', 'sClose', 'string', 'Close');",
            );
            assert!(message.contains("sOpen"), "{message}");
        }

        #[test]
        fn a_close_setting_of_the_wrong_type_stops_startup_naming_it() {
            let message = verb_failure(
                "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                 VALUES ('1:T.esm', 'sOpen', 'string', 'Open');
                 INSERT INTO game_settings (form_key, editor_id, value_type, value_float)
                 VALUES ('2:T.esm', 'sClose', 'float', 1.5);",
            );
            assert!(message.contains("sClose"), "{message}");
        }

        #[test]
        fn a_verb_without_a_value_or_blank_stops_startup_naming_it() {
            for value in ["NULL", "''", "'  '"] {
                let message = verb_failure(&format!(
                    "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                     VALUES ('1:T.esm', 'sOpen', 'string', {value}), ('2:T.esm', 'sClose', 'string', 'Close');"
                ));
                assert!(message.contains("sOpen"), "{value}: {message}");
            }
        }
    }

    fn prompts(entries: &[(&str, &str)]) -> DoorPrompts {
        prompts_with_silent(entries, &[])
    }

    fn prompts_with_silent(entries: &[(&str, &str)], silent: &[&str]) -> DoorPrompts {
        let silent = silent.iter().map(|source| (*source).to_string()).collect();
        DoorPrompts::new(labels_of(entries), silent, verbs())
    }

    fn entrances() -> WalkInTriggers {
        entrances_at(&[("near", 0.0), ("far", 2000.0)])
    }

    fn entrances_close_together() -> WalkInTriggers {
        entrances_at(&[("near", 0.0), ("far", 100.0)])
    }

    fn entrances_at(placed: &[(&str, f64)]) -> WalkInTriggers {
        use crate::assets::db::teleports::{DoorLocation, WalkInDoor};
        let entrance_at = |source_ref: &str, x: f64| WalkInDoor {
            source_ref: source_ref.to_string(),
            location: DoorLocation::Exterior {
                worldspace: "WS".to_string(),
            },
            position: [x, 0.0, 0.0],
            rotation: [0.0; 3],
            scale: 1.0,
        };
        let doors: Vec<WalkInDoor> = placed
            .iter()
            .map(|(source_ref, x)| entrance_at(source_ref, *x))
            .collect();
        WalkInTriggers::build(&doors, TEST_DISTANCES).0
    }

    /// Feet `units` Bethesda units from the origin of "near", straight along the ground.
    fn feet_at(units: f32) -> Vec3 {
        position_to_engine(units, 0.0, 0.0)
    }

    fn entrance_near(prompts: &DoorPrompts, units: f32) -> Option<ShownEntrance> {
        prompts.entrance(&entrances(), PlayerLocation::Exterior("WS"), feet_at(units))
    }

    fn name_only(name: &str) -> RolloverPrompt {
        RolloverPrompt {
            name: Arc::from(name),
            verb: None,
        }
    }

    #[test]
    fn the_nearest_entrance_within_the_distance_shows_its_label_without_a_verb() {
        let prompts = prompts(&[("near", "Diamond City"), ("far", "Elsewhere")]);
        assert_eq!(
            entrance_near(&prompts, 500.0),
            Some(ShownEntrance {
                prompt: Some(name_only("Diamond City"))
            })
        );
    }

    #[test]
    fn an_entrance_outside_the_distance_has_no_prompt_and_no_entrance() {
        let prompts = prompts(&[("near", "Diamond City")]);
        assert_eq!(entrance_near(&prompts, 550.1), None);
        assert!(entrance_near(&prompts, 549.9).is_some());
    }

    #[test]
    fn the_nearest_entrance_without_a_label_shows_nothing_even_if_the_next_one_has_one() {
        let prompts = prompts(&[("far", "Elsewhere")]);
        let entrance = prompts
            .entrance(
                &entrances_close_together(),
                PlayerLocation::Exterior("WS"),
                feet_at(0.0),
            )
            .unwrap();
        assert_eq!(entrance.prompt, None);
    }

    #[test]
    fn a_silent_entrance_shows_nothing_and_leaves_the_dot() {
        let prompts = prompts_with_silent(&[], &["near"]);
        let entrance = entrance_near(&prompts, 100.0);
        assert_eq!(entrance, None);
        assert_eq!(
            door_hud(false, None, entrance.as_ref()),
            DoorHud {
                activate: false,
                prompt: None
            }
        );
    }

    #[test]
    fn a_nearer_silent_entrance_decides_and_shows_nothing_though_a_farther_one_is_in_range() {
        let prompts = prompts_with_silent(&[("far", "Elsewhere")], &["near"]);
        let entrance = prompts.entrance(
            &entrances_close_together(),
            PlayerLocation::Exterior("WS"),
            feet_at(0.0),
        );
        assert_eq!(entrance, None);
        assert!(!door_hud(false, None, entrance.as_ref()).activate);
    }

    #[test]
    fn a_silent_entrance_shows_nothing_but_walking_in_still_starts_it() {
        let prompts = prompts_with_silent(&[], &["near"]);
        let mut triggers = entrances();
        let location = PlayerLocation::Exterior("WS");
        assert_eq!(prompts.entrance(&triggers, location, feet_at(100.0)), None);
        triggers.sample(location, Some(feet_at(-1000.0)));
        assert_eq!(
            triggers.sample(location, Some(feet_at(100.0))),
            Some("near")
        );
    }

    #[test]
    fn an_entrance_without_a_label_is_still_an_entrance_but_shows_nothing() {
        let entrance = entrance_near(&prompts(&[]), 100.0).unwrap();
        assert_eq!(entrance.prompt, None);
    }

    #[test]
    fn an_aimed_ordinary_prompt_wins_over_the_entrance() {
        let entrance = ShownEntrance {
            prompt: Some(name_only("Diamond City")),
        };
        let ordinary = RolloverPrompt {
            name: Arc::from("Door"),
            verb: Some(Arc::from("Open")),
        };
        assert_eq!(
            shown_prompt(Some(ordinary.clone()), Some(&entrance)),
            Some(ordinary)
        );
        assert_eq!(
            shown_prompt(None, Some(&entrance)),
            Some(name_only("Diamond City"))
        );
        assert_eq!(shown_prompt(None, None), None);
    }

    #[test]
    fn the_door_hud_combines_the_crosshair_and_the_prompt() {
        let ordinary = RolloverPrompt {
            name: Arc::from("Door"),
            verb: Some(Arc::from("Open")),
        };
        let labelled = ShownEntrance {
            prompt: Some(name_only("Diamond City")),
        };
        let unlabelled = ShownEntrance { prompt: None };
        let hud = |aimed, ordinary: Option<&RolloverPrompt>, entrance| {
            let hud = door_hud(aimed, ordinary.cloned(), entrance);
            (hud.activate, hud.prompt)
        };
        assert_eq!(
            hud(true, Some(&ordinary), Some(&labelled)),
            (true, Some(ordinary.clone()))
        );
        assert_eq!(
            hud(false, None, Some(&labelled)),
            (true, Some(name_only("Diamond City")))
        );
        assert_eq!(hud(false, None, Some(&unlabelled)), (true, None));
        assert_eq!(hud(true, None, None), (true, None));
        assert_eq!(hud(false, None, None), (false, None));
    }

    #[test]
    fn the_aimed_load_door_yields_its_label_with_the_open_verb() {
        let prompt = prompts(&[("d", "Museum of Freedom")])
            .prompt_for(Some("d"), |_| true, |_| None)
            .unwrap();
        assert_eq!(prompt.name.as_ref(), "Museum of Freedom");
        assert_eq!(prompt.verb.as_deref(), Some("Open"));
    }

    #[test]
    fn an_aimed_animated_door_yields_its_base_name_with_the_verb_by_state() {
        let name: Arc<str> = Arc::from("Door");
        let shown = |target| {
            prompts(&[])
                .prompt_for(Some("d"), |_| false, |_| Some(door(Some(&name), target)))
                .unwrap()
        };
        assert_eq!(shown(DoorState::Closed).verb.as_deref(), Some("Open"));
        assert_eq!(shown(DoorState::Open).verb.as_deref(), Some("Close"));
        assert_eq!(shown(DoorState::Open).name.as_ref(), "Door");
    }

    #[test]
    fn an_aimed_load_door_without_a_label_yields_no_prompt_even_though_it_is_activatable() {
        let name: Arc<str> = Arc::from("Door");
        let prompt = prompts(&[]).prompt_for(
            Some("d"),
            |_| true,
            |_| Some(door(Some(&name), DoorState::Closed)),
        );
        assert_eq!(prompt, None);
    }

    #[test]
    fn an_aimed_silent_load_door_shows_no_text_but_keeps_the_activate_crosshair() {
        let name: Arc<str> = Arc::from("Door");
        let prompt = prompts_with_silent(&[], &["d"]).prompt_for(
            Some("d"),
            |_| true,
            |_| Some(door(Some(&name), DoorState::Closed)),
        );
        assert_eq!(
            door_hud(true, prompt, None),
            DoorHud {
                activate: true,
                prompt: None
            }
        );
    }

    #[test]
    fn nothing_aimed_yields_no_prompt_and_asks_nothing() {
        let prompt = prompts(&[("d", "Cell")]).prompt_for(
            None,
            |_| panic!("no ref to ask about"),
            |_| panic!("no ref to ask about"),
        );
        assert_eq!(prompt, None);
    }

    #[test]
    fn an_empty_verb_setting_is_an_error_naming_it() {
        assert_eq!(
            PromptVerbs::new("Open", " ").err(),
            Some(VerbError::Empty {
                setting: CLOSE_SETTING
            })
        );
        assert_eq!(
            PromptVerbs::new("", "Close").err(),
            Some(VerbError::Empty {
                setting: OPEN_SETTING
            })
        );
    }
}
