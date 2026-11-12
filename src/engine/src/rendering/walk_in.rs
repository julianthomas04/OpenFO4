//! Walk-in entrances: automatic load doors. Walking into the display sphere of one makes the HUD
//! show where it leads; walking into its start sphere starts its transition. Both rules measure
//! the 3D distance from the player's feet to the door reference's origin against a radius derived
//! from the game's settings and the reference's scale, and both fire on entering, never on being
//! inside. Pure decision logic, no GPU or sim access: the settings, both distance rules, edge
//! arming and the location match all live here.

use anyhow::Context;
use glam::Vec3;

use crate::assets::db::DbClient;
use crate::assets::db::teleports::{DoorLocation, WalkInDoor};
use crate::bethesda_space::{
    UNITS_TO_ENGINE, position_from_engine, position_to_engine, rotation_to_engine,
};

pub(super) const DISPLAY_SETTING: &str = "fAutoDoorDisplayDistance";
pub(super) const START_SETTING: &str = "fAutoDoorActivateDistance";

/// Bethesda units: the settings for a reference at scale 1, or one reference's radii.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EntranceDistances {
    pub display_units: f32,
    pub start_units: f32,
}

impl EntranceDistances {
    /// A missing, non-numeric, non-positive or non-finite setting is an error naming it.
    pub(super) fn load(db: &DbClient) -> anyhow::Result<Self> {
        let distance = |setting: &'static str| {
            let value = db
                .get_game_setting_number(setting)?
                .with_context(|| format!("game setting {setting} is missing from the database"))?;
            positive_distance(setting, value)
        };
        Ok(Self {
            display_units: distance(DISPLAY_SETTING)?,
            start_units: distance(START_SETTING)?,
        })
    }

    /// The start sphere scales with the reference; the name shows within the settings' margin
    /// around it. A display radius below zero never shows.
    pub(super) fn scaled(self, scale: f32) -> Self {
        let start_units = self.start_units * scale.abs();
        Self {
            display_units: start_units + (self.display_units - self.start_units),
            start_units,
        }
    }
}

#[cfg(test)]
pub(super) const TEST_DISTANCES: EntranceDistances = EntranceDistances {
    display_units: 550.0,
    start_units: 350.0,
};

fn positive_distance(setting: &str, distance: f32) -> anyhow::Result<f32> {
    anyhow::ensure!(
        distance.is_finite() && distance > 0.0,
        "game setting {setting} is {distance}, not a positive distance"
    );
    Ok(distance)
}

/// The renderer's view of the player this frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlayState {
    pub walking: bool,
    pub in_game: bool,
    pub pipboy_open: bool,
    pub teleport_pending: bool,
}

impl PlayState {
    /// Whether the player's position means anything to walk-in entrances: only while walking in
    /// play with the Pip-Boy closed, not while a load is pending, runs or fades in. The start and
    /// the entrance prompt share it.
    pub(super) fn walk_in_live(self) -> bool {
        self.walking && self.in_game && !self.pipboy_open && !self.teleport_pending
    }
}

/// Where the player currently is, as the streamer reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlayerLocation<'a> {
    Interior(&'a str),
    Exterior(&'a str),
}

impl PlayerLocation<'_> {
    fn is(self, door_location: &DoorLocation) -> bool {
        match (self, door_location) {
            (Self::Interior(cell), DoorLocation::Interior { cell: door_cell }) => cell == door_cell,
            (
                Self::Exterior(worldspace),
                DoorLocation::Exterior {
                    worldspace: door_worldspace,
                },
            ) => worldspace == door_worldspace,
            _ => false,
        }
    }
}

struct Entrance {
    source_ref: String,
    /// Bethesda units.
    origin: Vec3,
    limits: EntranceDistances,
    /// Engine-space ground direction out of the doorway, for the dev hook's walk line.
    outward: Vec3,
}

impl Entrance {
    /// `None` for a non-finite position, rotation or scale.
    fn new(door: &WalkInDoor, distances: EntranceDistances) -> Option<Self> {
        let finite = door
            .position
            .iter()
            .chain(&door.rotation)
            .chain([&door.scale])
            .all(|value| value.is_finite());
        if !finite {
            return None;
        }
        let [rx, ry, rz] = door.rotation.map(|angle| angle as f32);
        let facing = rotation_to_engine(rx, ry, rz) * Vec3::Z;
        Some(Self {
            source_ref: door.source_ref.clone(),
            origin: Vec3::from(door.position.map(|v| v as f32)),
            limits: distances.scaled(door.scale as f32),
            outward: Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::X),
        })
    }

    /// The game's `getdistance`; both rules and the order among entrances measure with it.
    fn distance_units(&self, feet: Vec3) -> f32 {
        feet.distance(self.origin)
    }

    fn shows_at(&self, feet: Vec3) -> bool {
        self.distance_units(feet) <= self.limits.display_units
    }

    fn starts_at(&self, feet: Vec3) -> bool {
        self.distance_units(feet) <= self.limits.start_units
    }

    /// Whether the name shows at `feet` after the move from `previous`: inside the display sphere
    /// now, and either shown already or outside it before. Only the end point matters.
    fn shown_after(&self, was_shown: bool, previous: Vec3, feet: Vec3) -> bool {
        self.shows_at(feet) && (was_shown || !self.shows_at(previous))
    }

    /// Outside at `previous`, and inside at `feet` or the straight path there touches the start
    /// sphere, so one long frame cannot jump across a small one.
    fn entered(&self, previous: Vec3, feet: Vec3) -> bool {
        !self.starts_at(previous)
            && (self.starts_at(feet)
                || self.path_distance_units(previous, feet) <= self.limits.start_units)
    }

    /// The closest the path from `from` to `to` comes to the origin.
    fn path_distance_units(&self, from: Vec3, to: Vec3) -> f32 {
        let path = to - from;
        let along = (self.origin - from).dot(path) / path.length_squared();
        let fraction = if along.is_finite() {
            along.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (from + path * fraction).distance(self.origin)
    }
}

struct LocationEntrances {
    location: DoorLocation,
    entrances: Vec<Entrance>,
}

/// Doors reported once at boot.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct BuildReport {
    /// No entrance: a non-finite position, rotation or scale.
    pub non_finite: Vec<String>,
    /// The start sphere shrinks to the origin.
    pub zero_scale: Vec<String>,
}

/// How the player stands relative to an entrance of the current location, for the diagnostic
/// trace. Distances are Bethesda units.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EntranceProbe {
    pub source_ref: String,
    pub distance_units: f32,
    pub limits: EntranceDistances,
    pub shown: bool,
    pub inside_start: bool,
    /// Whether the last sample had a previous position to compare with.
    pub armed: bool,
}

/// Where a dev hook may put the player to walk into an entrance, engine space.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WalkLine {
    /// Outside the start zone, in front of the doorway.
    pub approach: Vec3,
    /// The door's origin; a move from `approach` enters the start zone.
    pub through: Vec3,
}

/// How far beyond the start zone the approach sits, Bethesda units.
const WALK_LINE_CLEARANCE_UNITS: f32 = 30.0;

/// What the last valid sample left behind: where the player was and which entrances show.
struct LastSample {
    location_index: usize,
    /// Bethesda units.
    feet: Vec3,
    /// Per entrance of the location, in build order.
    shown: Vec<bool>,
}

/// Every walk-in entrance plus the edge state deciding when one shows and starts.
#[derive(Default)]
pub(super) struct WalkInTriggers {
    locations: Vec<LocationEntrances>,
    /// `None` until a baseline is taken: after [`Self::rearm`], in a new location, or without
    /// entrances. Nothing shows then.
    previous: Option<LastSample>,
    armed: bool,
}

impl WalkInTriggers {
    pub(super) fn build(doors: &[WalkInDoor], distances: EntranceDistances) -> (Self, BuildReport) {
        let mut report = BuildReport::default();
        let mut locations: Vec<LocationEntrances> = Vec::new();
        for door in doors {
            let Some(entrance) = Entrance::new(door, distances) else {
                report.non_finite.push(door.source_ref.clone());
                continue;
            };
            if door.scale == 0.0 {
                report.zero_scale.push(door.source_ref.clone());
            }
            match locations
                .iter_mut()
                .find(|entry| entry.location == door.location)
            {
                Some(entry) => entry.entrances.push(entrance),
                None => locations.push(LocationEntrances {
                    location: door.location.clone(),
                    entrances: vec![entrance],
                }),
            }
        }
        let triggers = Self {
            locations,
            previous: None,
            armed: false,
        };
        (triggers, report)
    }

    fn entrances_of(&self, location: PlayerLocation<'_>) -> Option<(usize, &[Entrance])> {
        self.locations
            .iter()
            .enumerate()
            .find(|(_, entry)| location.is(&entry.location))
            .map(|(index, entry)| (index, entry.entrances.as_slice()))
    }

    /// The entrances of `location` with whether each shows its name as of the last sample; none
    /// shows without a baseline taken in `location`.
    fn entrances_with_display(
        &self,
        location: PlayerLocation<'_>,
    ) -> Option<(&[Entrance], &[bool])> {
        let (index, entrances) = self.entrances_of(location)?;
        let shown = self
            .previous
            .as_ref()
            .filter(|last| last.location_index == index)
            .map_or(&[][..], |last| last.shown.as_slice());
        Some((entrances, shown))
    }

    /// The next valid sample is only a baseline and never starts or shows anything: call after
    /// anything that puts the player somewhere new, so arriving inside a start or display zone is
    /// not entering it.
    pub(super) fn rearm(&mut self) {
        self.previous = None;
    }

    /// The source ref of the entrance the player just walked into on the way from the previous
    /// valid sample to `feet` (engine space), and the update of which names show. A sample
    /// without feet (player not spawned) changes nothing. The first sample after [`Self::rearm`]
    /// or in a new location is only a baseline. When several zones are entered in one sample the
    /// first built wins.
    pub(super) fn sample(
        &mut self,
        location: PlayerLocation<'_>,
        feet: Option<Vec3>,
    ) -> Option<&str> {
        let feet = position_from_engine(feet?);
        let Some((index, _)) = self.entrances_of(location) else {
            self.previous = None;
            self.armed = false;
            return None;
        };
        let entrances = &self.locations[index].entrances;
        let before = self
            .previous
            .take()
            .filter(|last| last.location_index == index);
        self.armed = before.is_some();
        let shown = match &before {
            Some(last) => entrances
                .iter()
                .zip(&last.shown)
                .map(|(entrance, was_shown)| entrance.shown_after(*was_shown, last.feet, feet))
                .collect(),
            None => vec![false; entrances.len()],
        };
        let started = before.and_then(|last| {
            entrances
                .iter()
                .find(|entrance| entrance.entered(last.feet, feet))
        });
        self.previous = Some(LastSample {
            location_index: index,
            feet,
            shown,
        });
        started.map(|entrance| entrance.source_ref.as_str())
    }

    /// The source ref of the entrance of `location` that decides what the HUD shows at `feet`
    /// (engine space): the nearest one showing its name as of the last sample.
    pub(super) fn nearest_entrance(
        &self,
        location: PlayerLocation<'_>,
        feet: Vec3,
    ) -> Option<&str> {
        let feet = position_from_engine(feet);
        let (entrances, shown) = self.entrances_with_display(location)?;
        shown_entrance(entrances, shown, feet).map(|entrance| entrance.source_ref.as_str())
    }

    /// The probe of the entrance that decides the HUD at `feet` (engine space), else of the
    /// nearest one; `None` when `location` has no entrances.
    pub(super) fn probe(&self, location: PlayerLocation<'_>, feet: Vec3) -> Option<EntranceProbe> {
        let feet = position_from_engine(feet);
        let (entrances, shown) = self.entrances_with_display(location)?;
        let shown = shown_entrance(entrances, shown, feet);
        let entrance = shown.or_else(|| nearest(entrances.iter(), feet))?;
        Some(EntranceProbe {
            source_ref: entrance.source_ref.clone(),
            distance_units: entrance.distance_units(feet),
            limits: entrance.limits,
            shown: shown.is_some(),
            inside_start: entrance.starts_at(feet),
            armed: self.armed,
        })
    }

    pub(super) fn walk_line(&self, source_ref: &str) -> Option<WalkLine> {
        let entrance = self
            .locations
            .iter()
            .flat_map(|entry| &entry.entrances)
            .find(|entrance| entrance.source_ref == source_ref)?;
        let origin = position_to_engine(entrance.origin.x, entrance.origin.y, entrance.origin.z);
        let reach = (entrance.limits.start_units + WALK_LINE_CLEARANCE_UNITS) * UNITS_TO_ENGINE;
        Some(WalkLine {
            approach: origin + entrance.outward * reach,
            through: origin,
        })
    }
}

/// The entrance nearest to `feet` (Bethesda units) among `candidates`, the first built winning a
/// tie.
fn nearest<'a>(candidates: impl Iterator<Item = &'a Entrance>, feet: Vec3) -> Option<&'a Entrance> {
    candidates.min_by(|first, second| {
        first
            .distance_units(feet)
            .total_cmp(&second.distance_units(feet))
    })
}

/// The one selection the HUD and the trace share: the nearest entrance among those showing.
fn shown_entrance<'a>(
    entrances: &'a [Entrance],
    shown: &[bool],
    feet: Vec3,
) -> Option<&'a Entrance> {
    let showing = entrances
        .iter()
        .zip(shown)
        .filter_map(|(entrance, is_shown)| is_shown.then_some(entrance));
    nearest(showing, feet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISTANCES: EntranceDistances = TEST_DISTANCES;

    fn door_at(source_ref: &str, location: DoorLocation, position: [f64; 3]) -> WalkInDoor {
        WalkInDoor {
            source_ref: source_ref.to_string(),
            location,
            position,
            rotation: [0.0; 3],
            scale: 1.0,
        }
    }

    fn exterior() -> DoorLocation {
        DoorLocation::Exterior {
            worldspace: "WS".to_string(),
        }
    }

    fn interior(cell: &str) -> DoorLocation {
        DoorLocation::Interior {
            cell: cell.to_string(),
        }
    }

    fn door(source_ref: &str, location: DoorLocation) -> WalkInDoor {
        door_at(source_ref, location, [0.0; 3])
    }

    fn build(doors: &[WalkInDoor]) -> WalkInTriggers {
        WalkInTriggers::build(doors, DISTANCES).0
    }

    fn one_door() -> WalkInTriggers {
        build(&[door("DOOR", exterior())])
    }

    /// Engine-space feet at Bethesda `(x, y, z)`.
    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        position_to_engine(x, y, z)
    }

    const FAR: [f32; 3] = [2000.0, 0.0, 0.0];
    const INSIDE: [f32; 3] = [100.0, 0.0, 0.0];

    fn sample(triggers: &mut WalkInTriggers, [x, y, z]: [f32; 3]) -> Option<&str> {
        triggers.sample(PlayerLocation::Exterior("WS"), Some(at(x, y, z)))
    }

    /// Baselines at `from`, then samples at `to`: what the player does in one step.
    fn step(triggers: &mut WalkInTriggers, from: [f32; 3], to: [f32; 3]) -> Option<&str> {
        triggers.rearm();
        sample(triggers, from);
        sample(triggers, to)
    }

    fn nearest(triggers: &WalkInTriggers, [x, y, z]: [f32; 3]) -> Option<String> {
        triggers
            .nearest_entrance(PlayerLocation::Exterior("WS"), at(x, y, z))
            .map(str::to_owned)
    }

    /// What shows at `to` after walking there from far outside every sphere.
    fn shown_on_entering(triggers: &mut WalkInTriggers, to: [f32; 3]) -> Option<String> {
        step(triggers, FAR, to);
        nearest(triggers, to)
    }

    #[test]
    fn entering_the_start_zone_fires() {
        let mut triggers = one_door();
        assert_eq!(sample(&mut triggers, FAR), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn staying_inside_fires_once() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    #[test]
    fn starting_inside_does_not_fire_until_leaving_and_returning() {
        let mut triggers = one_door();
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, FAR), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn arriving_inside_after_a_rearm_does_not_start_until_leaving_and_reentering() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        triggers.rearm();
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, FAR), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    fn small_door() -> WalkInTriggers {
        build(&[WalkInDoor {
            scale: 0.23,
            ..door("DOOR", exterior())
        }])
    }

    #[test]
    fn a_path_across_the_start_sphere_starts_once_though_both_ends_lie_outside() {
        let mut triggers = small_door();
        sample(&mut triggers, [-300.0, 0.0, 0.0]);
        assert_eq!(sample(&mut triggers, [300.0, 0.0, 0.0]), Some("DOOR"));
        assert_eq!(sample(&mut triggers, [300.0, 0.0, 0.0]), None);
        assert_eq!(sample(&mut triggers, [600.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_path_passing_beside_the_start_sphere_does_not_start() {
        let mut triggers = small_door();
        sample(&mut triggers, [-300.0, 81.0, 0.0]);
        assert_eq!(sample(&mut triggers, [300.0, 81.0, 0.0]), None);
        sample(&mut triggers, [-300.0, 0.0, 81.0]);
        assert_eq!(sample(&mut triggers, [300.0, 0.0, 81.0]), None);
    }

    #[test]
    fn no_feet_never_fire_and_keep_the_previous_edge_state() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(triggers.sample(PlayerLocation::Exterior("WS"), None), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn the_start_counts_height() {
        let mut triggers = one_door();
        assert_eq!(
            step(&mut triggers, [0.0, 0.0, 2000.0], [0.0, 0.0, 351.0]),
            None
        );
        assert_eq!(step(&mut triggers, FAR, [0.0, 0.0, 349.0]), Some("DOOR"));
        assert_eq!(step(&mut triggers, FAR, [210.0, 0.0, 279.9]), Some("DOOR"));
        assert_eq!(step(&mut triggers, FAR, [210.0, 0.0, 280.1]), None);
    }

    #[test]
    fn the_display_counts_height() {
        let mut triggers = one_door();
        for shown in [[549.0, 0.0, 0.0], [0.0, 0.0, 549.0], [330.0, 0.0, 439.9]] {
            assert_eq!(
                shown_on_entering(&mut triggers, shown),
                Some("DOOR".into()),
                "{shown:?}"
            );
        }
        for hidden in [[551.0, 0.0, 0.0], [0.0, 0.0, -551.0], [330.0, 440.1, 0.0]] {
            assert_eq!(shown_on_entering(&mut triggers, hidden), None, "{hidden:?}");
        }
    }

    #[test]
    fn walking_into_the_display_sphere_shows_the_name() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(nearest(&triggers, FAR), None);
        sample(&mut triggers, [540.0, 0.0, 0.0]);
        assert_eq!(nearest(&triggers, [540.0, 0.0, 0.0]), Some("DOOR".into()));
    }

    #[test]
    fn the_name_stays_while_the_player_remains_inside_the_display_sphere() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        for inside in [
            [540.0, 0.0, 0.0],
            [540.0, 0.0, 0.0],
            [400.0, 0.0, 0.0],
            INSIDE,
        ] {
            sample(&mut triggers, inside);
            assert_eq!(
                nearest(&triggers, inside),
                Some("DOOR".into()),
                "{inside:?}"
            );
        }
    }

    #[test]
    fn leaving_the_display_sphere_hides_the_name_and_reentering_shows_it_again() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        sample(&mut triggers, [540.0, 0.0, 0.0]);
        sample(&mut triggers, [560.0, 0.0, 0.0]);
        assert_eq!(nearest(&triggers, [560.0, 0.0, 0.0]), None);
        sample(&mut triggers, [540.0, 0.0, 0.0]);
        assert_eq!(nearest(&triggers, [540.0, 0.0, 0.0]), Some("DOOR".into()));
    }

    #[test]
    fn arriving_inside_the_display_sphere_shows_nothing_until_leaving_and_reentering() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        triggers.rearm();
        for inside in [INSIDE, INSIDE, [400.0, 0.0, 0.0]] {
            sample(&mut triggers, inside);
            assert_eq!(nearest(&triggers, inside), None, "{inside:?}");
        }
        sample(&mut triggers, FAR);
        sample(&mut triggers, INSIDE);
        assert_eq!(nearest(&triggers, INSIDE), Some("DOOR".into()));
    }

    #[test]
    fn the_first_sample_of_a_fresh_trigger_set_inside_the_display_sphere_shows_nothing() {
        let mut triggers = one_door();
        sample(&mut triggers, INSIDE);
        assert_eq!(nearest(&triggers, INSIDE), None);
    }

    #[test]
    fn nothing_shows_between_a_rearm_and_the_next_sample() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        sample(&mut triggers, INSIDE);
        triggers.rearm();
        assert_eq!(nearest(&triggers, INSIDE), None);
    }

    #[test]
    fn a_sample_without_feet_keeps_what_shows() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        sample(&mut triggers, INSIDE);
        triggers.sample(PlayerLocation::Exterior("WS"), None);
        assert_eq!(nearest(&triggers, INSIDE), Some("DOOR".into()));
    }

    #[test]
    fn a_sample_without_feet_keeps_an_arrival_inside_silent() {
        let mut triggers = one_door();
        sample(&mut triggers, INSIDE);
        triggers.sample(PlayerLocation::Exterior("WS"), None);
        sample(&mut triggers, INSIDE);
        assert_eq!(nearest(&triggers, INSIDE), None);
    }

    #[test]
    fn only_the_end_of_a_move_decides_the_display() {
        let mut triggers = small_door();
        sample(&mut triggers, [-300.0, 0.0, 0.0]);
        sample(&mut triggers, [300.0, 0.0, 0.0]);
        assert_eq!(nearest(&triggers, [300.0, 0.0, 0.0]), None);
    }

    #[test]
    fn each_entrance_keeps_its_own_display_edge_where_the_spheres_overlap() {
        let mut triggers = build(&[
            door("A", exterior()),
            door_at("B", exterior(), [600.0, 0.0, 0.0]),
        ]);
        let in_both = [300.0, 0.0, 0.0];
        let in_a_only = [0.0, 0.0, 0.0];
        sample(&mut triggers, in_both);
        sample(&mut triggers, in_a_only);
        sample(&mut triggers, in_both);
        assert_eq!(nearest(&triggers, in_both), Some("B".into()));
    }

    #[test]
    fn a_sample_in_another_location_hides_the_names_of_the_previous_one() {
        let mut triggers = build(&[door("OUT", exterior()), door("IN", interior("A"))]);
        let inside = Some(at(INSIDE[0], INSIDE[1], INSIDE[2]));
        sample(&mut triggers, FAR);
        sample(&mut triggers, INSIDE);
        triggers.sample(PlayerLocation::Interior("A"), inside);
        assert_eq!(nearest(&triggers, INSIDE), None);
    }

    #[test]
    fn both_limits_count_their_boundary_as_inside() {
        let entrance = Entrance::new(&door("DOOR", exterior()), DISTANCES).unwrap();
        assert!(entrance.shows_at(Vec3::new(550.0, 0.0, 0.0)));
        assert!(!entrance.shows_at(Vec3::new(550.01, 0.0, 0.0)));
        assert!(entrance.starts_at(Vec3::new(0.0, 0.0, 350.0)));
        assert!(!entrance.starts_at(Vec3::new(0.0, 0.0, 350.01)));
    }

    #[test]
    fn the_start_radius_scales_and_the_display_radius_keeps_the_settings_margin_beyond_it() {
        for (scale, start, display) in [
            (1.0, 350.0, 550.0),
            (0.889_999_985_7, 311.5, 511.5),
            (0.23, 80.5, 280.5),
            (-0.5, 175.0, 375.0),
        ] {
            let radii = DISTANCES.scaled(scale as f32);
            assert!(
                (radii.start_units - start).abs() < 1e-3,
                "{scale}: {radii:?}"
            );
            assert!(
                (radii.display_units - display).abs() < 1e-3,
                "{scale}: {radii:?}"
            );
        }
    }

    #[test]
    fn a_small_reference_shows_and_starts_within_its_scaled_radii() {
        let mut triggers = small_door();
        assert_eq!(
            shown_on_entering(&mut triggers, [280.0, 0.0, 0.0]),
            Some("DOOR".into())
        );
        assert_eq!(shown_on_entering(&mut triggers, [281.0, 0.0, 0.0]), None);
        assert_eq!(step(&mut triggers, FAR, [80.0, 0.0, 0.0]), Some("DOOR"));
        assert_eq!(step(&mut triggers, FAR, [81.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_display_setting_far_below_the_start_setting_never_shows() {
        let settings = EntranceDistances {
            display_units: 100.0,
            start_units: 350.0,
        };
        let half = WalkInDoor {
            scale: 0.5,
            ..door("DOOR", exterior())
        };
        assert!(settings.scaled(0.5).display_units < 0.0);
        let (mut triggers, _) = WalkInTriggers::build(&[half], settings);
        assert_eq!(shown_on_entering(&mut triggers, [0.0; 3]), None);
        assert_eq!(step(&mut triggers, FAR, INSIDE), Some("DOOR"));
    }

    #[test]
    fn a_negative_scale_starts_by_its_size() {
        let mirrored = WalkInDoor {
            scale: -0.5,
            ..door("DOOR", exterior())
        };
        let mut triggers = build(&[mirrored]);
        assert_eq!(step(&mut triggers, FAR, [174.0, 0.0, 0.0]), Some("DOOR"));
        assert_eq!(step(&mut triggers, FAR, [176.0, 0.0, 0.0]), None);
        assert_eq!(
            shown_on_entering(&mut triggers, [374.0, 0.0, 0.0]),
            Some("DOOR".into())
        );
        assert_eq!(shown_on_entering(&mut triggers, [376.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_zero_scale_is_reported_and_starts_only_on_a_path_that_meets_its_origin() {
        let flat = WalkInDoor {
            scale: 0.0,
            ..door("FLAT", exterior())
        };
        let (mut triggers, report) = WalkInTriggers::build(&[flat], DISTANCES);
        assert_eq!(report.zero_scale, ["FLAT"]);
        assert_eq!(report.non_finite, Vec::<String>::new());
        assert_eq!(step(&mut triggers, FAR, [1.0, 0.0, 0.0]), None);
        assert_eq!(
            step(&mut triggers, [-100.0, 0.0, 1.0], [100.0, 0.0, 1.0]),
            None
        );
        assert_eq!(step(&mut triggers, FAR, [0.0; 3]), Some("FLAT"));
        assert_eq!(
            shown_on_entering(&mut triggers, [199.0, 0.0, 0.0]),
            Some("FLAT".into())
        );
        assert_eq!(shown_on_entering(&mut triggers, [201.0, 0.0, 0.0]), None);
    }

    #[test]
    fn the_rules_measure_from_the_origin_whatever_the_rotation() {
        let turned = WalkInDoor {
            rotation: [0.4, 0.0, 1.2],
            ..door_at("DOOR", exterior(), [1000.0, -500.0, 200.0])
        };
        let mut triggers = build(&[turned]);
        assert_eq!(
            shown_on_entering(&mut triggers, [1000.0, 49.0, 200.0]),
            Some("DOOR".into())
        );
        assert_eq!(
            shown_on_entering(&mut triggers, [1000.0, 51.0, 200.0]),
            None
        );
        assert_eq!(
            step(&mut triggers, FAR, [1000.0, -500.0, -149.0]),
            Some("DOOR")
        );
    }

    #[test]
    fn the_nearest_entrance_in_3d_shows() {
        let doors = [
            door_at("below", exterior(), [0.0, 0.0, -450.0]),
            door_at("level", exterior(), [400.0, 0.0, 0.0]),
        ];
        let mut triggers = build(&doors);
        assert_eq!(
            shown_on_entering(&mut triggers, [0.0; 3]),
            Some("level".into())
        );
    }

    #[test]
    fn of_equally_near_entrances_the_first_built_shows() {
        let first = door_at("first", exterior(), [300.0, 0.0, 0.0]);
        let second = door_at("second", exterior(), [-300.0, 0.0, 0.0]);
        let mut built = build(&[first.clone(), second.clone()]);
        let mut reversed = build(&[second, first]);
        assert_eq!(
            shown_on_entering(&mut built, [0.0; 3]),
            Some("first".into())
        );
        assert_eq!(
            shown_on_entering(&mut reversed, [0.0; 3]),
            Some("second".into())
        );
    }

    #[test]
    fn entering_overlapping_start_zones_at_once_starts_the_first_built_only() {
        let mut triggers = build(&[
            door("A", exterior()),
            door_at("B", exterior(), [200.0, 0.0, 0.0]),
        ]);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, [100.0, 0.0, 0.0]), Some("A"));
        assert_eq!(sample(&mut triggers, [-300.0, 0.0, 0.0]), None);
        assert_eq!(sample(&mut triggers, [500.0, 0.0, 0.0]), Some("B"));
    }

    #[test]
    fn the_nearest_entrance_decides_only_where_its_own_display_radius_reaches() {
        let small = WalkInDoor {
            scale: 0.23,
            ..door_at("small", exterior(), [0.0, 0.0, 0.0])
        };
        let mut triggers = build(&[small, door_at("large", exterior(), [600.0, 0.0, 0.0])]);
        assert_eq!(
            shown_on_entering(&mut triggers, [250.0, 0.0, 0.0]),
            Some("small".into())
        );
        assert_eq!(
            shown_on_entering(&mut triggers, [290.0, 0.0, 0.0]),
            Some("large".into())
        );
    }

    #[test]
    fn entrances_of_another_location_are_neither_shown_nor_started() {
        let mut triggers = build(&[
            door("OUT", exterior()),
            door("IN_A", interior("A")),
            door("IN_B", interior("B")),
        ]);
        let inside = Some(at(INSIDE[0], INSIDE[1], INSIDE[2]));
        let far = Some(at(FAR[0], FAR[1], FAR[2]));
        triggers.sample(PlayerLocation::Interior("A"), far);
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("A"), inside),
            Some("IN_A")
        );
        assert_eq!(triggers.sample(PlayerLocation::Interior("C"), inside), None);
        assert_eq!(
            triggers.nearest_entrance(PlayerLocation::Interior("C"), at(0.0, 0.0, 0.0)),
            None
        );
    }

    #[test]
    fn arriving_inside_in_another_location_does_not_start_until_leaving_and_reentering() {
        let mut triggers = build(&[door("OUT", exterior()), door("IN", interior("A"))]);
        let inside = Some(at(INSIDE[0], INSIDE[1], INSIDE[2]));
        let far = Some(at(FAR[0], FAR[1], FAR[2]));
        triggers.sample(PlayerLocation::Exterior("WS"), far);
        assert_eq!(triggers.sample(PlayerLocation::Interior("A"), inside), None);
        assert_eq!(triggers.sample(PlayerLocation::Interior("A"), inside), None);
        assert_eq!(triggers.sample(PlayerLocation::Interior("A"), far), None);
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("A"), inside),
            Some("IN")
        );
    }

    #[test]
    fn each_entrance_keeps_its_own_edge() {
        let mut triggers = build(&[
            door("FIRST", exterior()),
            door_at("SECOND", exterior(), [5000.0, 0.0, 0.0]),
        ]);
        let at_second = [5100.0, 0.0, 0.0];
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("FIRST"));
        assert_eq!(sample(&mut triggers, at_second), Some("SECOND"));
        assert_eq!(sample(&mut triggers, at_second), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("FIRST"));
    }

    #[test]
    fn a_door_with_non_finite_input_gets_no_entrance() {
        let broken = door_at("BROKEN", exterior(), [f64::NAN, 0.0, 0.0]);
        let (mut triggers, report) = WalkInTriggers::build(&[broken], DISTANCES);
        assert_eq!(report.non_finite, ["BROKEN"]);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    #[test]
    fn the_walk_line_starts_outside_and_enters_the_start_zone() {
        let turned = WalkInDoor {
            rotation: [0.0, 0.0, 0.7],
            ..door_at("DOOR", exterior(), [300.0, 400.0, 0.0])
        };
        let mut triggers = build(&[turned]);
        let line = triggers.walk_line("DOOR").unwrap();
        let location = PlayerLocation::Exterior("WS");
        triggers.rearm();
        assert_eq!(triggers.sample(location, Some(line.approach)), None);
        assert_eq!(triggers.sample(location, Some(line.through)), Some("DOOR"));
    }

    #[test]
    fn the_walk_line_approaches_from_the_side_the_door_faces() {
        let reach = DISTANCES.start_units + WALK_LINE_CLEARANCE_UNITS;
        for (rotation_z, expected) in [
            (0.0, Vec3::new(0.0, -reach, 0.0)),
            (std::f64::consts::FRAC_PI_2, Vec3::new(-reach, 0.0, 0.0)),
        ] {
            let facing = WalkInDoor {
                rotation: [0.0, 0.0, rotation_z],
                ..door_at("DOOR", exterior(), [300.0, 400.0, 50.0])
            };
            let line = build(&[facing]).walk_line("DOOR").unwrap();
            let offset = position_from_engine(line.approach) - Vec3::new(300.0, 400.0, 50.0);
            assert!(
                offset.distance(expected) < 0.05,
                "{rotation_z}: {offset} != {expected}"
            );
            assert!(
                position_from_engine(line.through).distance(Vec3::new(300.0, 400.0, 50.0)) < 0.05
            );
        }
    }

    #[test]
    fn an_unknown_door_has_no_walk_line() {
        assert_eq!(one_door().walk_line("NOPE"), None);
    }

    fn probe_at(triggers: &WalkInTriggers, [x, y, z]: [f32; 3]) -> Option<EntranceProbe> {
        triggers.probe(PlayerLocation::Exterior("WS"), at(x, y, z))
    }

    /// A half-size door above the ground (radii 375 and 175) and one far away.
    fn half_size_door_above() -> WalkInTriggers {
        let half = WalkInDoor {
            scale: 0.5,
            ..door_at("NEAR", exterior(), [0.0, 0.0, 300.0])
        };
        build(&[half, door_at("FAR", exterior(), [3000.0, 0.0, 0.0])])
    }

    #[test]
    fn the_probe_reports_the_3d_distance_and_the_radii_of_the_reference() {
        let probe = probe_at(&half_size_door_above(), [400.0, 0.0, 0.0]).unwrap();
        assert_eq!(probe.source_ref, "NEAR");
        assert!((probe.distance_units - 500.0).abs() < 0.01, "{probe:?}");
        assert_eq!(
            probe.limits,
            EntranceDistances {
                display_units: 375.0,
                start_units: 175.0,
            }
        );
    }

    #[test]
    fn the_probe_reports_whether_the_entrance_shows() {
        let mut triggers = half_size_door_above();
        step(&mut triggers, FAR, [400.0, 0.0, 0.0]);
        assert!(!probe_at(&triggers, [400.0, 0.0, 0.0]).unwrap().shown);
        step(&mut triggers, FAR, [200.0, 0.0, 0.0]);
        assert!(probe_at(&triggers, [200.0, 0.0, 0.0]).unwrap().shown);
    }

    #[test]
    fn the_probe_reports_an_entrance_arrived_inside_of_as_not_shown() {
        let mut triggers = half_size_door_above();
        triggers.rearm();
        sample(&mut triggers, [200.0, 0.0, 0.0]);
        assert!(!probe_at(&triggers, [200.0, 0.0, 0.0]).unwrap().shown);
    }

    #[test]
    fn the_probe_reports_whether_the_feet_are_inside_the_start_zone() {
        let triggers = half_size_door_above();
        assert!(!probe_at(&triggers, [200.0, 0.0, 0.0]).unwrap().inside_start);
        assert!(
            probe_at(&triggers, [100.0, 0.0, 300.0])
                .unwrap()
                .inside_start
        );
    }

    #[test]
    fn the_probe_reports_whether_the_last_sample_was_armed() {
        let mut triggers = half_size_door_above();
        sample(&mut triggers, FAR);
        assert!(!probe_at(&triggers, FAR).unwrap().armed);
        sample(&mut triggers, [400.0, 0.0, 0.0]);
        assert!(probe_at(&triggers, [400.0, 0.0, 0.0]).unwrap().armed);
    }

    #[test]
    fn the_probe_takes_the_entrance_the_hud_shows_over_a_nearer_one_out_of_its_reach() {
        let small = WalkInDoor {
            scale: 0.23,
            ..door_at("small", exterior(), [0.0, 0.0, 0.0])
        };
        let mut triggers = build(&[small, door_at("large", exterior(), [600.0, 0.0, 0.0])]);
        step(&mut triggers, FAR, [290.0, 0.0, 0.0]);
        let probe = probe_at(&triggers, [290.0, 0.0, 0.0]).unwrap();
        assert_eq!(probe.source_ref, "large");
        assert!(probe.shown);
    }

    #[test]
    fn with_nothing_shown_the_probe_reports_the_nearest_entrance() {
        let probe = probe_at(&half_size_door_above(), [-1000.0, 0.0, 0.0]).unwrap();
        assert_eq!(probe.source_ref, "NEAR");
        assert!(!probe.shown);
    }

    #[test]
    fn a_location_without_entrances_has_no_probe() {
        assert_eq!(
            half_size_door_above().probe(PlayerLocation::Interior("A"), Vec3::ZERO),
            None
        );
    }

    #[test]
    fn walk_in_is_live_only_while_walking_in_game_with_the_pipboy_closed_and_no_teleport_pending() {
        let live = PlayState {
            walking: true,
            in_game: true,
            pipboy_open: false,
            teleport_pending: false,
        };
        assert!(live.walk_in_live());
        for blocked in [
            PlayState {
                walking: false,
                ..live
            },
            PlayState {
                in_game: false,
                ..live
            },
            PlayState {
                pipboy_open: true,
                ..live
            },
            PlayState {
                teleport_pending: true,
                ..live
            },
        ] {
            assert!(!blocked.walk_in_live(), "{blocked:?}");
        }
    }

    #[test]
    fn no_doors_never_fire() {
        let mut triggers = WalkInTriggers::default();
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    mod real_install {
        use super::*;
        use crate::assets::db::test_support::installed_database;

        /// The HUD lets the nearest entrance decide; which of two entrances in range wins cannot be
        /// observed while no two display spheres of one space overlap.
        #[test]
        #[ignore = "needs a converted installation (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
        fn no_two_entrances_of_one_space_have_overlapping_display_spheres() {
            let db = installed_database();
            let doors = db.get_walk_in_doors().unwrap().doors;
            let distances = EntranceDistances::load(&db).unwrap();
            let (triggers, _) = WalkInTriggers::build(&doors, distances);
            let mut overlapping = Vec::new();
            for entry in &triggers.locations {
                for (index, first) in entry.entrances.iter().enumerate() {
                    for second in &entry.entrances[index + 1..] {
                        let reach = first.limits.display_units.max(0.0)
                            + second.limits.display_units.max(0.0);
                        if first.origin.distance(second.origin) <= reach {
                            overlapping.push((first.source_ref.clone(), second.source_ref.clone()));
                        }
                    }
                }
            }
            assert_eq!(overlapping, Vec::<(String, String)>::new());
        }
    }

    mod settings {
        use super::*;
        use crate::assets::db::test_support::database_with;

        fn setting(editor_id: &str, value: &str) -> String {
            format!(
                "INSERT INTO game_settings (form_key, editor_id, value_type, value_float)
                 VALUES ('{editor_id}:T.esm', '{editor_id}', 'float', {value});"
            )
        }

        fn load(statements: &str) -> anyhow::Result<EntranceDistances> {
            EntranceDistances::load(&database_with(statements))
        }

        #[test]
        fn both_settings_are_read() {
            let distances = load(&format!(
                "{}{}",
                setting(DISPLAY_SETTING, "550.0"),
                setting(START_SETTING, "350.0")
            ))
            .unwrap();
            assert_eq!(distances, DISTANCES);
        }

        #[test]
        fn a_missing_or_unusable_setting_is_an_error_naming_it() {
            for (name, other) in [
                (DISPLAY_SETTING, START_SETTING),
                (START_SETTING, DISPLAY_SETTING),
            ] {
                let usable = setting(other, "400.0");
                let cases = [
                    String::new(),
                    format!(
                        "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
                         VALUES ('x:T.esm', '{name}', 'string', 'far');"
                    ),
                    setting(name, "0.0"),
                    setting(name, "-5.0"),
                    format!(
                        "INSERT INTO game_settings (form_key, editor_id, value_type)
                         VALUES ('x:T.esm', '{name}', 'float');"
                    ),
                ];
                for case in cases {
                    let message = format!("{:#}", load(&format!("{usable}{case}")).unwrap_err());
                    assert!(message.contains(name), "{case}: {message}");
                }
            }
        }

        #[test]
        fn a_non_finite_distance_is_rejected() {
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let error = positive_distance(START_SETTING, bad)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(START_SETTING), "{bad}: {error}");
            }
        }
    }
}
