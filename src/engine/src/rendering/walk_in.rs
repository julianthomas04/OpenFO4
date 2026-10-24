//! Walk-in load doors: automatic DOOR refs teleport the player when the player's capsule
//! enters their trigger volume. Pure decision logic, no GPU or sim access: edge arming, the
//! box-versus-capsule test and the location match all live here.

use glam::{Quat, Vec3};

use crate::assets::db::teleports::{DoorLocation, WalkInDoor};
use crate::bethesda_space::{
    UNITS_TO_ENGINE, axes_to_engine, position_to_engine, rotation_to_engine,
};
use crate::sim::PlayerBody;

/// Smallest trigger half-extent per axis, in engine units. A door whose OBND collapses on an
/// axis (zero or inverted extent) would never be entered; this keeps the box well-formed while
/// staying below the capsule radius, so the capsule inflation still dominates the reach.
pub(super) const MIN_HALF_EXTENT: f32 = 0.5;

/// Bethesda units below which an OBND extent counts as collapsed.
const DEGENERATE_EXTENT_UNITS: f64 = 0.01;

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

/// An oriented box in engine space.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Obb {
    center: Vec3,
    rotation: Quat,
    half_extents: Vec3,
}

impl Obb {
    /// Whether the capsule touches the box: the capsule's core segment against the box grown by
    /// the capsule radius (touching counts). Corners are treated as square, which errs towards
    /// firing by less than the radius.
    fn touches_capsule(&self, body: &PlayerBody) -> bool {
        let radius = body.radius;
        let lowest = radius.min(body.height * 0.5);
        let highest = (body.height - radius).max(body.height * 0.5);
        let bottom = body.feet + Vec3::Y * lowest;
        let top = body.feet + Vec3::Y * highest;
        let inverse = self.rotation.inverse();
        segment_touches_box(
            inverse * (bottom - self.center),
            inverse * (top - self.center),
            self.half_extents + Vec3::splat(radius),
        )
    }
}

/// Slab test of the segment `a`..`b` against the origin-centred axis-aligned box.
fn segment_touches_box(a: Vec3, b: Vec3, half_extents: Vec3) -> bool {
    let direction = b - a;
    let mut entry = 0.0_f32;
    let mut exit = 1.0_f32;
    for axis in 0..3 {
        let (start, delta, half) = (a[axis], direction[axis], half_extents[axis]);
        if delta.abs() < f32::EPSILON {
            if start.abs() > half {
                return false;
            }
            continue;
        }
        let (near, far) = ((-half - start) / delta, (half - start) / delta);
        entry = entry.max(near.min(far));
        exit = exit.min(near.max(far));
        if entry > exit {
            return false;
        }
    }
    true
}

struct Trigger {
    source_ref: String,
    volume: Obb,
}

struct LocationTriggers {
    location: DoorLocation,
    triggers: Vec<Trigger>,
    inside: Vec<bool>,
}

/// Why a door got no usable trigger or a corrected one; reported once at boot.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct BuildReport {
    /// Refs with a collapsed OBND axis, widened to [`MIN_HALF_EXTENT`].
    pub degenerate_bounds: Vec<String>,
    /// Refs with a non-finite position, rotation, scale or bounds, left without a trigger.
    pub non_finite: Vec<String>,
}

/// Every walk-in trigger volume plus the edge state deciding when one fires.
#[derive(Default)]
pub(super) struct WalkInTriggers {
    locations: Vec<LocationTriggers>,
    active_location: Option<usize>,
    baseline_pending: bool,
}

impl WalkInTriggers {
    pub(super) fn build(doors: &[WalkInDoor]) -> (Self, BuildReport) {
        let mut report = BuildReport::default();
        let mut locations: Vec<LocationTriggers> = Vec::new();
        for door in doors {
            let Some((volume, degenerate)) = volume_of(door) else {
                report.non_finite.push(door.source_ref.clone());
                continue;
            };
            if degenerate {
                report.degenerate_bounds.push(door.source_ref.clone());
            }
            let index = locations
                .iter()
                .position(|entry| entry.location == door.location)
                .unwrap_or_else(|| {
                    locations.push(LocationTriggers {
                        location: door.location.clone(),
                        triggers: Vec::new(),
                        inside: Vec::new(),
                    });
                    locations.len() - 1
                });
            locations[index].triggers.push(Trigger {
                source_ref: door.source_ref.clone(),
                volume,
            });
            locations[index].inside.push(false);
        }
        let triggers = Self {
            locations,
            active_location: None,
            baseline_pending: true,
        };
        (triggers, report)
    }

    /// The next valid sample only records where the player stands and never fires: call after
    /// anything that puts the player somewhere new, so arriving inside a trigger is not entering it.
    pub(super) fn rearm(&mut self) {
        self.baseline_pending = true;
    }

    /// The source ref of the door whose trigger the player just walked into, if any. `body` is
    /// `None` while the player is not spawned; such a sample changes nothing. When several
    /// triggers are entered in one sample the first built wins. Switching location also takes a
    /// baseline: edge state from another location says nothing about this one.
    pub(super) fn sample(
        &mut self,
        location: PlayerLocation<'_>,
        body: Option<&PlayerBody>,
    ) -> Option<&str> {
        let body = body?;
        let Some(index) = self
            .locations
            .iter()
            .position(|entry| location.is(&entry.location))
        else {
            self.active_location = None;
            self.baseline_pending = true;
            return None;
        };
        let baseline =
            std::mem::take(&mut self.baseline_pending) || self.active_location != Some(index);
        self.active_location = Some(index);

        let entry = &mut self.locations[index];
        let mut fired = None;
        for (slot, trigger) in entry.triggers.iter().enumerate() {
            let now = trigger.volume.touches_capsule(body);
            let was = std::mem::replace(&mut entry.inside[slot], now);
            if now && !was && !baseline && fired.is_none() {
                fired = Some(slot);
            }
        }
        fired.map(|slot| entry.triggers[slot].source_ref.as_str())
    }
}

/// The engine-space trigger box of a door, and whether its OBND had to be widened. `None` for
/// non-finite input.
fn volume_of(door: &WalkInDoor) -> Option<(Obb, bool)> {
    let finite = door
        .position
        .iter()
        .chain(&door.rotation)
        .chain(&door.bounds.min)
        .chain(&door.bounds.max)
        .chain([&door.scale])
        .all(|value| value.is_finite());
    if !finite {
        return None;
    }
    let scale = door.scale as f32;
    let [min, max] = [door.bounds.min, door.bounds.max].map(|point| point.map(|v| v as f32));
    let local_center = axes_to_engine(
        f32::midpoint(min[0], max[0]),
        f32::midpoint(min[1], max[1]),
        f32::midpoint(min[2], max[2]),
    ) * (UNITS_TO_ENGINE * scale);
    let degenerate = (0..3).any(|axis| f64::from(max[axis] - min[axis]) < DEGENERATE_EXTENT_UNITS);
    let half_extents = axes_to_engine(
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    )
    .abs()
        * (UNITS_TO_ENGINE * scale.abs());
    let rotation = rotation_to_engine(
        door.rotation[0] as f32,
        door.rotation[1] as f32,
        door.rotation[2] as f32,
    );
    let position = position_to_engine(
        door.position[0] as f32,
        door.position[1] as f32,
        door.position[2] as f32,
    );
    Some((
        Obb {
            center: position + rotation * local_center,
            rotation,
            half_extents: half_extents.max(Vec3::splat(MIN_HALF_EXTENT)),
        },
        degenerate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::teleports::DoorBounds;

    const RADIUS: f32 = 2.5;
    const HEIGHT: f32 = 12.6;

    /// A 100 x 20 x 200 unit door-local box (10 x 2 x 20 engine units) at the origin, which
    /// stands 10 engine units wide along engine X, 2 deep along engine Z, 20 tall.
    fn door(source_ref: &str, location: DoorLocation) -> WalkInDoor {
        WalkInDoor {
            source_ref: source_ref.to_string(),
            location,
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0],
            scale: 1.0,
            bounds: DoorBounds {
                min: [-50.0, -10.0, 0.0],
                max: [50.0, 10.0, 200.0],
            },
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

    fn body_at(feet: Vec3) -> PlayerBody {
        PlayerBody {
            feet,
            height: HEIGHT,
            radius: RADIUS,
        }
    }

    const FAR: Vec3 = Vec3::new(100.0, 0.0, 0.0);
    const INSIDE: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    fn one_door() -> WalkInTriggers {
        WalkInTriggers::build(&[door("DOOR", exterior())]).0
    }

    fn sample(triggers: &mut WalkInTriggers, feet: Vec3) -> Option<&str> {
        triggers.sample(PlayerLocation::Exterior("WS"), Some(&body_at(feet)))
    }

    #[test]
    fn fires_when_the_player_walks_in() {
        let mut triggers = one_door();
        assert_eq!(sample(&mut triggers, FAR), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn fires_once_while_the_player_stays_inside() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    #[test]
    fn a_player_arriving_inside_does_not_fire_until_leaving_and_returning() {
        let mut triggers = one_door();
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, FAR), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn fires_again_after_exit_and_re_entry() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn rearm_turns_the_next_sample_into_a_baseline_that_never_fires() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        triggers.rearm();
        assert_eq!(sample(&mut triggers, INSIDE), None);
        assert_eq!(sample(&mut triggers, INSIDE), None);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn rearm_baseline_waits_for_a_valid_sample() {
        let mut triggers = one_door();
        triggers.rearm();
        assert_eq!(triggers.sample(PlayerLocation::Exterior("WS"), None), None);
        assert_eq!(sample(&mut triggers, INSIDE), None);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn a_trigger_built_but_never_sampled_starts_as_a_baseline() {
        let mut triggers = one_door();
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    #[test]
    fn no_body_never_fires_and_keeps_the_previous_edge_state() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(triggers.sample(PlayerLocation::Exterior("WS"), None), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("DOOR"));
    }

    #[test]
    fn doors_of_other_locations_are_ignored() {
        let mut triggers = WalkInTriggers::build(&[
            door("OUT", exterior()),
            door("IN_A", interior("A")),
            door("IN_B", interior("B")),
        ])
        .0;
        let body = body_at(INSIDE);
        let far = body_at(FAR);
        triggers.sample(PlayerLocation::Interior("A"), Some(&far));
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("A"), Some(&body)),
            Some("IN_A")
        );
        triggers.sample(PlayerLocation::Interior("A"), Some(&far));
        assert_eq!(
            triggers.sample(PlayerLocation::Exterior("OTHER"), Some(&body)),
            None
        );
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("C"), Some(&body)),
            None
        );
    }

    #[test]
    fn switching_location_takes_a_baseline_instead_of_firing() {
        let mut triggers =
            WalkInTriggers::build(&[door("OUT", exterior()), door("IN", interior("A"))]).0;
        let inside = body_at(INSIDE);
        triggers.sample(PlayerLocation::Exterior("WS"), Some(&body_at(FAR)));
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("A"), Some(&inside)),
            None
        );
        assert_eq!(
            triggers.sample(PlayerLocation::Interior("A"), Some(&inside)),
            None
        );
    }

    #[test]
    fn each_trigger_keeps_its_own_edge() {
        let mut second = door("SECOND", exterior());
        second.position = [0.0, 2000.0, 0.0];
        let mut triggers = WalkInTriggers::build(&[door("FIRST", exterior()), second]).0;
        let at_second = Vec3::new(0.0, 0.0, -200.0);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("FIRST"));
        assert_eq!(sample(&mut triggers, at_second), Some("SECOND"));
        assert_eq!(sample(&mut triggers, at_second), None);
        assert_eq!(sample(&mut triggers, INSIDE), Some("FIRST"));
    }

    #[test]
    fn capsule_touching_the_box_from_the_side_counts() {
        let mut triggers = one_door();
        // Box spans x in [-5, 5]; the capsule reaches 2.5 further.
        sample(&mut triggers, FAR);
        assert_eq!(
            sample(&mut triggers, Vec3::new(7.5, 0.0, 0.0)),
            Some("DOOR")
        );
    }

    #[test]
    fn capsule_just_short_of_the_box_does_not_count() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, Vec3::new(7.51, 0.0, 0.0)), None);
    }

    #[test]
    fn capsule_above_or_below_the_box_does_not_count() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        // Box is 20 tall from y = 0; the capsule core starts RADIUS above the feet.
        assert_eq!(sample(&mut triggers, Vec3::new(0.0, 25.0, 0.0)), None);
        assert_eq!(
            sample(&mut triggers, Vec3::new(0.0, -HEIGHT - 1.0, 0.0)),
            None
        );
        assert_eq!(
            sample(&mut triggers, Vec3::new(0.0, 20.0, 0.0)),
            Some("DOOR")
        );
    }

    #[test]
    fn a_body_reaching_the_box_with_its_head_only_counts() {
        let mut triggers = one_door();
        sample(&mut triggers, FAR);
        // Feet 10 units under the box floor: the body spans y in [-10, 2.6], reaching the box.
        assert_eq!(
            sample(&mut triggers, Vec3::new(0.0, -10.0, 0.0)),
            Some("DOOR")
        );
    }

    #[test]
    fn a_rotated_box_is_tested_along_its_rotated_axes() {
        // A quarter turn about Bethesda Z swaps the box's engine X and Z extents: the 5-unit
        // half-width now runs along Z and the 1-unit half-depth along X.
        let mut rotated = door("DOOR", exterior());
        rotated.rotation = [0.0, 0.0, std::f32::consts::FRAC_PI_2.into()];
        let mut triggers = WalkInTriggers::build(&[rotated]).0;
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, Vec3::new(4.0, 0.0, 0.0)), None);
        assert_eq!(
            sample(&mut triggers, Vec3::new(0.0, 0.0, 7.0)),
            Some("DOOR")
        );
    }

    #[test]
    fn position_and_scale_move_and_grow_the_box() {
        let mut scaled = door("DOOR", exterior());
        scaled.position = [300.0, -200.0, 100.0];
        scaled.scale = 2.0;
        let mut triggers = WalkInTriggers::build(&[scaled]).0;
        // Bethesda (300, -200, 100) is engine (30, 10, 20). The box is 20 wide in X after
        // scaling: x in [20, 40], reach 2.5 further.
        sample(&mut triggers, FAR);
        assert_eq!(
            sample(&mut triggers, Vec3::new(17.5, 10.0, 20.0)),
            Some("DOOR")
        );
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, Vec3::new(17.4, 10.0, 20.0)), None);
    }

    #[test]
    fn a_collapsed_bound_is_widened_and_reported() {
        let mut flat = door("FLAT", exterior());
        flat.bounds = DoorBounds {
            min: [-50.0, 0.0, 0.0],
            max: [50.0, 0.0, 200.0],
        };
        let (mut triggers, report) = WalkInTriggers::build(&[flat, door("FINE", interior("A"))]);
        assert_eq!(report.degenerate_bounds, ["FLAT"]);
        assert_eq!(report.non_finite, Vec::<String>::new());
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), Some("FLAT"));
        let widened = triggers.locations[0].triggers[0].volume.half_extents;
        assert!((widened.z - MIN_HALF_EXTENT).abs() < 1e-6, "{widened:?}");
    }

    #[test]
    fn a_door_with_non_finite_input_gets_no_trigger() {
        let mut broken = door("BROKEN", exterior());
        broken.position = [f64::NAN, 0.0, 0.0];
        let (mut triggers, report) = WalkInTriggers::build(&[broken]);
        assert_eq!(report.non_finite, ["BROKEN"]);
        sample(&mut triggers, FAR);
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }

    #[test]
    fn no_doors_never_fire() {
        let mut triggers = WalkInTriggers::default();
        assert_eq!(sample(&mut triggers, INSIDE), None);
    }
}
