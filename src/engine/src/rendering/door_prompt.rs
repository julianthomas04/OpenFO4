//! Text shown while aiming at a door: the name of the place a load door leads to (or the door's
//! own name) and the verb of the activation. Pure rules; the callers own the data loading.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;

use super::animated_objects::{AnimatedDoor, DoorState};
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
    pub verb: Arc<str>,
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

/// What the load door shows: its own name when its base carries the no-to-text flag, else the
/// destination's name. `None` when that name is empty.
fn load_door_label(
    teleport: &TeleportRecord,
    regions: &MapRegionIndex,
) -> Result<Option<Arc<str>>, DoorFlagsError> {
    let shows_own_name = match teleport.source_door_flags.as_deref() {
        None => false,
        flags => flags_have_no_to_text(&format!("door ref {}", teleport.source_ref), flags)?,
    };
    let name = if shows_own_name {
        non_blank(teleport.source_door_name.as_deref())
    } else {
        destination_name(teleport, regions)
    };
    Ok(name.map(Arc::from))
}

/// The labels of the load doors that show one, by source ref, plus the doors whose flags could
/// not be read (they get no label).
#[derive(Default)]
pub(super) struct LoadDoorLabels {
    pub labels: HashMap<String, Arc<str>>,
    pub rejected: Vec<DoorFlagsError>,
}

pub(super) fn load_door_labels(
    teleports: &[TeleportRecord],
    regions: &MapRegionIndex,
) -> LoadDoorLabels {
    let mut result = LoadDoorLabels::default();
    for teleport in teleports {
        match load_door_label(teleport, regions) {
            Ok(Some(label)) => {
                result.labels.insert(teleport.source_ref.clone(), label);
            }
            Ok(None) => {}
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
            verb: Arc::clone(&verbs.open),
        });
    }
    let door = animated_door?;
    Some(RolloverPrompt {
        name: Arc::clone(door.name?),
        verb: Arc::clone(verbs.for_target(door.target)),
    })
}

/// Everything the prompt of an aimed door is made of, resolved once at startup.
pub(super) struct DoorPrompts {
    labels: HashMap<String, Arc<str>>,
    verbs: PromptVerbs,
}

impl DoorPrompts {
    pub(super) fn new(labels: HashMap<String, Arc<str>>, verbs: PromptVerbs) -> Self {
        Self { labels, verbs }
    }

    /// A missing or unusable verb setting stops startup. Unreadable map regions degrade to none,
    /// reported: exterior doors then show their worldspace's name. A database from an older installer lacks cell names and region map entries,
    /// which the schema reports once.
    pub(super) fn load(db: &DbClient, teleports: &[TeleportRecord]) -> Result<Self, StartupError> {
        let verbs = Self::load_verbs(db).map_err(StartupError::DoorPromptVerbs)?;
        let regions = match db.get_map_regions() {
            Ok(regions) => MapRegionIndex::new(regions),
            Err(error) => {
                tracing::warn!("map regions not loaded ({error:#}); no door shows a map region");
                MapRegionIndex::default()
            }
        };
        let LoadDoorLabels { labels, rejected } = load_door_labels(teleports, &regions);
        if !rejected.is_empty() {
            let refs: Vec<String> = rejected.iter().map(ToString::to_string).collect();
            tracing::warn!(
                "{} load doors show no name: {}",
                rejected.len(),
                refs.join("; ")
            );
        }
        tracing::info!(
            "door prompts: {} of {} load doors have a name",
            labels.len(),
            teleports.len()
        );
        Ok(Self::new(labels, verbs))
    }

    fn load_verbs(db: &DbClient) -> anyhow::Result<PromptVerbs> {
        let setting = |name: &'static str| {
            db.get_game_setting_string(name)?
                .with_context(|| format!("game setting {name} is missing from the database"))
        };
        Ok(PromptVerbs::new(
            &setting(OPEN_SETTING)?,
            &setting(CLOSE_SETTING)?,
        )?)
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
            source_door_name: Some("Door".to_string()),
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

    fn flagged(mut teleport: TeleportRecord, name: Option<&str>) -> TeleportRecord {
        teleport.source_door_flags = Some("64".to_string());
        teleport.source_door_name = name.map(str::to_string);
        teleport
    }

    fn label(teleport: &TeleportRecord, regions: Vec<MapRegion>) -> Option<String> {
        load_door_label(teleport, &MapRegionIndex::new(regions))
            .unwrap()
            .map(|label| label.to_string())
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
        assert_eq!(
            label(&flagged(interior(Some("Cell")), Some("  ")), Vec::new()),
            None
        );
    }

    #[test]
    fn a_flagged_load_door_shows_its_base_name_instead_of_the_destination() {
        let teleport = flagged(interior(Some("Museum of Freedom")), Some("Hatch"));
        assert_eq!(label(&teleport, Vec::new()), Some("Hatch".to_string()));
    }

    #[test]
    fn a_nameless_flagged_load_door_has_no_label() {
        let teleport = flagged(interior(Some("Museum of Freedom")), None);
        assert_eq!(label(&teleport, Vec::new()), None);
    }

    #[test]
    fn a_nameless_load_door_without_the_flag_still_shows_its_destination() {
        let mut teleport = interior(Some("Museum of Freedom"));
        teleport.source_door_name = None;
        assert_eq!(
            label(&teleport, Vec::new()),
            Some("Museum of Freedom".to_string())
        );
    }

    #[test]
    fn a_load_door_without_a_base_record_shows_its_destination() {
        let mut teleport = interior(Some("Museum of Freedom"));
        teleport.source_door_name = None;
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
    fn doors_without_a_label_are_absent_from_the_label_map() {
        let mut nameless = interior(None);
        nameless.source_ref = "nameless".to_string();
        let mut named = interior(Some("Cell"));
        named.source_ref = "named".to_string();

        let result = load_door_labels(&[nameless, named], &MapRegionIndex::default());

        assert_eq!(result.labels.keys().collect::<Vec<_>>(), ["named"]);
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
            assert_eq!(prompt.verb.as_ref(), "Close", "{target:?}");
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
                    verb: Arc::from("Open")
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
        assert_eq!(prompt.verb.as_ref(), "Open");
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
        use std::path::{Path, PathBuf};

        use super::*;
        use crate::assets::db::DbClient;
        use crate::assets::layout::data_dir;
        use crate::assets::{AssetRootEnv, resolve_asset_root};

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

        fn database() -> DbClient {
            let path = std::env::var_os("OPENFO4_DB").map_or_else(
                || {
                    let root = resolve_asset_root(
                        &AssetRootEnv::from_process(),
                        Path::new(env!("CARGO_MANIFEST_DIR")),
                    )
                    .expect("set OPENFO4_INSTALL_DIR to a converted installation");
                    PathBuf::from(format!("{}/data.db", data_dir(root.to_str().unwrap())))
                },
                PathBuf::from,
            );
            DbClient::new(path.to_str().unwrap()).unwrap()
        }

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
                assert_eq!(shown, expected, "{source_ref}");
            }
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
                Some(error @ StartupError::DoorPromptVerbs(_)) => error.to_string(),
                other => panic!("expected the verb startup error, got {other:?}"),
            }
        }

        fn shown(prompts: &DoorPrompts, source_ref: &str) -> Option<(String, String)> {
            prompts
                .prompt_for(Some(source_ref), |_| true, |_| None)
                .map(|prompt| (prompt.name.to_string(), prompt.verb.to_string()))
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
        DoorPrompts::new(labels_of(entries), verbs())
    }

    #[test]
    fn the_aimed_load_door_yields_its_label_with_the_open_verb() {
        let prompt = prompts(&[("d", "Museum of Freedom")])
            .prompt_for(Some("d"), |_| true, |_| None)
            .unwrap();
        assert_eq!(prompt.name.as_ref(), "Museum of Freedom");
        assert_eq!(prompt.verb.as_ref(), "Open");
    }

    #[test]
    fn an_aimed_animated_door_yields_its_base_name_with_the_verb_by_state() {
        let name: Arc<str> = Arc::from("Door");
        let shown = |target| {
            prompts(&[])
                .prompt_for(Some("d"), |_| false, |_| Some(door(Some(&name), target)))
                .unwrap()
        };
        assert_eq!(shown(DoorState::Closed).verb.as_ref(), "Open");
        assert_eq!(shown(DoorState::Open).verb.as_ref(), "Close");
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
