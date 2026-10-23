//! The character's contact with dynamic bodies. The kinematic controller walks
//! on and slides along static geometry only; dynamic bodies are handled here
//! the way Jolt's `CharacterVirtual` does: they block the horizontal motion, the
//! character shoves them with an impulse derived from its intended velocity
//! (not from the swept remainder, which is nil once the cast stops short), and
//! it stands on one only when it lands on top of it, loading it with its weight
//! as a bounded force (Jolt's `mMass`), never with an impulse from fall speed.

use std::sync::OnceLock;

use glam::Vec3;
use rapier3d::parry::query::details::ShapeCastOptions;
use rapier3d::parry::query::{Contact, contact};
use rapier3d::prelude::*;

use super::mirror::dynamic_scene_query_filter;
use super::physics::PhysicsContext;
use crate::utils::warn_once;

const TRACE_ENV_VAR: &str = "OPENFO4_PLAYER_PUSH_TRACE";
/// Share of the closing speed a shove removes per step (Jolt's `cDamping`).
const PUSH_DAMPING: f32 = 0.9;
/// Share of an overlap resolved per step, as outward velocity on the character
/// and as extra closing speed on the body (Jolt's `cPenetrationResolution`).
const PENETRATION_RESOLUTION: f32 = 0.4;
/// Contacts are gathered this many skins out so a stopped character keeps seeing the body.
const CONTACT_PREDICTION_SKINS: f32 = 1.5;
/// Height above the feet, in skins, below which a body is ground rather than an obstacle.
const FOOT_BAND_SKINS: f32 = 2.0;

fn trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(TRACE_ENV_VAR).is_ok_and(|value| value == "1"))
}

pub(super) struct Character<'a> {
    pub capsule: &'a Capsule,
    pub mass: f32,
    /// Most force the character pushes a body with.
    pub push_force: f32,
    pub gravity: f32,
    pub skin: f32,
}

/// What the character stands on: how far it can drop and the body carrying it.
pub(super) struct Support {
    pub drop: f32,
    normal_y: f32,
    bone: bool,
    body: RigidBodyHandle,
    point: Vec3,
}

/// One player step as the trace sees it, for `trace_step`.
pub(super) struct StepTrace<'a> {
    pub start: Vec3,
    pub end: Vec3,
    pub eye_above_center: f32,
    pub static_translation: Vec3,
    pub allowed_horizontal: Vec3,
    pub static_grounded: bool,
    pub grounded: bool,
    pub support: Option<&'a Support>,
}

/// `OPENFO4_PLAYER_PUSH_TRACE` line for a step in which the dynamic pass did
/// something: the whole-step displacement, vertical part included, and the eye.
pub(super) fn trace_step(step: &StepTrace) {
    if !trace_enabled() {
        return;
    }
    let horizontal = Vec3::new(step.static_translation.x, 0.0, step.static_translation.z);
    if step.support.is_none() && step.allowed_horizontal == horizontal {
        return;
    }
    let moved = step.end - step.start;
    tracing::info!(
        "player step: moved=({:.3}, {:.3}, {:.3}) static_h={:.3} allowed_h={:.3} eye_y={:.3} static_grounded={} grounded={} support={}",
        moved.x,
        moved.y,
        moved.z,
        horizontal.length(),
        step.allowed_horizontal.length(),
        step.end.y + step.eye_above_center,
        step.static_grounded,
        step.grounded,
        step.support.map_or_else(
            || "none".to_owned(),
            |support| format!(
                "drop={:.3} normal_y={:.2} bone={}",
                support.drop, support.normal_y, support.bone
            ),
        ),
    );
}

/// A dynamic body touching the character above its feet, seen from the
/// character: `normal` is horizontal and points into the body.
struct BodyContact {
    body: RigidBodyHandle,
    normal: Vec3,
    /// Signed gap between the surfaces, negative while they overlap.
    gap: f32,
}

/// Contact between the capsule at `pose` and `collider` within `prediction`.
/// A shape pair parry cannot handle is reported once and treated as no contact.
fn capsule_contact(
    character: &Character,
    pose: &Pose,
    collider: &Collider,
    prediction: f32,
) -> Option<Contact> {
    match contact(
        pose,
        character.capsule,
        collider.position(),
        collider.shape(),
        prediction,
    ) {
        Ok(contact) => contact,
        Err(error) => {
            warn_once("player_dynamic_contact_unsupported", || {
                format!(
                    "player: dynamic-body contact unsupported ({error}); that body will not block the player"
                )
            });
            None
        }
    }
}

/// Every dynamic body within the skin of the capsule at `start`. Contacts at
/// foot level are ground, not obstacles. Re-evaluated each step rather than
/// only swept, so a body pushed into the capsule (or one the capsule already
/// overlaps) keeps blocking after the sweep stops reporting it.
fn contacts_around(
    physics: &PhysicsContext,
    character: &Character,
    start: Vec3,
) -> Vec<BodyContact> {
    let pose = Pose::from_translation(start);
    let prediction = character.skin * CONTACT_PREDICTION_SKINS;
    let feet = start.y - character.capsule.half_height() - character.capsule.radius;
    let aabb = character.capsule.compute_aabb(&pose).loosened(prediction);
    let queries = physics
        .world
        .query_pipeline_with_filter(dynamic_scene_query_filter());
    queries
        .intersect_aabb_conservative(aabb)
        .filter_map(|(_, collider)| {
            let body = collider.parent()?;
            let contact = capsule_contact(character, &pose, collider, prediction)?;
            let normal = Vec3::new(contact.normal1.x, 0.0, contact.normal1.z).normalize_or_zero();
            (contact.point1.y > feet + FOOT_BAND_SKINS * character.skin && normal != Vec3::ZERO)
                .then_some(BodyContact {
                    body,
                    normal,
                    gap: contact.dist,
                })
        })
        .collect()
}

/// Resolves the character's horizontal motion against dynamic bodies, after
/// Jolt's `CharacterVirtual`: every body in contact is a blocking constraint that
/// limits how far the motion may close in on it (`approach_limit`), overlap
/// becomes a bounded outward velocity inside that same constraint rather than a
/// separate push, and each contacted body is shoved. Bodies still ahead are
/// caught by a sweep that stops at the skin.
pub(super) fn shove_and_slide(
    physics: &mut PhysicsContext,
    character: &Character,
    start: Vec3,
    horizontal: Vec3,
    dt: f32,
) -> Vec3 {
    let contacts = contacts_around(physics, character, start);
    let mut allowed = horizontal;
    for contact in &contacts {
        let excess = allowed.dot(contact.normal) - approach_limit(contact.gap, character.skin);
        if excess > 0.0 {
            allowed -= contact.normal * excess;
        }
        let overlap = (-contact.gap).max(0.0);
        let speed = horizontal.dot(contact.normal) / dt + overlap * PENETRATION_RESOLUTION / dt;
        let max_impulse = character.push_force * dt;
        if let Some(impulse) = shove_impulse(
            physics,
            contact.body,
            character.mass,
            contact.normal,
            speed,
            max_impulse,
        ) {
            physics.shove_body(contact.body, impulse, None);
        }
    }
    let moved = sweep_and_slide(physics, character, start, allowed);
    if trace_enabled() {
        for contact in &contacts {
            trace_contact(
                physics,
                contact,
                horizontal.dot(contact.normal),
                moved.dot(contact.normal),
            );
        }
    }
    moved
}

/// How far the character may move toward a body `gap` away: up to the skin
/// when clear of it, nowhere inside the skin, and out at a fraction of the
/// overlap while penetrating (Jolt folds the same bias into the contact
/// velocity: `mDistance * mPenetrationRecoverySpeed / dt`). Being gap-aware, not
/// an on/off block, is what keeps the character from stop-start stuttering at
/// the edge of the contact band.
fn approach_limit(gap: f32, skin: f32) -> f32 {
    if gap >= skin {
        gap - skin
    } else if gap >= 0.0 {
        0.0
    } else {
        gap * PENETRATION_RESOLUTION
    }
}

fn trace_contact(physics: &PhysicsContext, contact: &BodyContact, intended: f32, moved: f32) {
    let Some(body) = physics.world.bodies.get(contact.body) else {
        return;
    };
    tracing::info!(
        "player push: bone={} mass={:.2} gap={:.3} intended={intended:.3} moved={moved:.3} body_speed={:.2} sleeping={}",
        body.user_data != 0,
        body.mass(),
        contact.gap,
        body.linvel().dot(contact.normal),
        body.is_sleeping(),
    );
}

/// Moves `motion` until a dynamic body is a skin away, then slides along it.
fn sweep_and_slide(
    physics: &PhysicsContext,
    character: &Character,
    start: Vec3,
    motion: Vec3,
) -> Vec3 {
    let Some((direction, distance)) = normalize_with_length(motion) else {
        return motion;
    };
    // Lifted clear of whatever the feet rest on, so a body under them is ground, not a wall.
    let origin = start + Vec3::Y * FOOT_BAND_SKINS * character.skin;
    let hit = physics
        .world
        .query_pipeline_with_filter(dynamic_scene_query_filter())
        .cast_shape(
            &Pose::from_translation(origin),
            direction,
            character.capsule,
            ShapeCastOptions {
                target_distance: character.skin,
                stop_at_penetration: false,
                max_time_of_impact: distance,
                compute_impact_geometry_on_penetration: true,
            },
        );
    let Some((_, hit)) = hit else {
        return motion;
    };
    let away = -Vec3::new(hit.normal1.x, 0.0, hit.normal1.z).normalize_or_zero();
    let away = if away.dot(direction) > 0.0 {
        away
    } else {
        direction
    };
    let advance = direction * hit.time_of_impact;
    let remaining = motion - advance;
    advance + remaining - away * remaining.dot(away).max(0.0)
}

/// Impulse through the body's centre of mass that closes the speed gap along
/// `away`, with the character's mass in series so the body never gains more
/// than the gap; none when the body already moves that fast. Acting on the
/// centre of mass keeps the shove a translation: at the contact point most of
/// it spins a long, floor-pinned bone up to the angular speed cap, which then
/// reads as "already moving at that speed" and the bone never leaves. The
/// impulse is capped at `max_impulse`, the most force the character can push
/// with for one step (Jolt's `mMaxStrength * dt`).
fn shove_impulse(
    physics: &PhysicsContext,
    body: RigidBodyHandle,
    character_mass: f32,
    away: Vec3,
    speed: f32,
    max_impulse: f32,
) -> Option<Vec3> {
    let body = physics.world.bodies.get(body)?;
    let closing = (speed - body.linvel().dot(away)) * PUSH_DAMPING;
    let inverse_mass = body.mass_properties().effective_inv_mass.dot(away * away);
    (closing > 0.0 && inverse_mass > 0.0)
        .then(|| away * (closing / (inverse_mass + 1.0 / character_mass)).min(max_impulse))
}

/// The dynamic body the character stands on: below `start` within `reach`, with
/// a walkable surface (normal at most `max_slope` off vertical) touching the
/// capsule at foot level. A body against the side of the lower hemisphere is
/// not support, so it is never loaded with the character's weight.
pub(super) fn support_below(
    physics: &PhysicsContext,
    character: &Character,
    start: Vec3,
    reach: f32,
    max_slope: f32,
) -> Option<Support> {
    let (collider, hit) = physics
        .world
        .query_pipeline_with_filter(dynamic_scene_query_filter())
        .cast_shape(
            &Pose::from_translation(start),
            -Vec3::Y,
            character.capsule,
            ShapeCastOptions {
                target_distance: character.skin,
                stop_at_penetration: false,
                max_time_of_impact: reach,
                compute_impact_geometry_on_penetration: true,
            },
        )?;
    let pose = Pose::from_translation(start - Vec3::Y * hit.time_of_impact);
    let feet = pose.translation.y - character.capsule.half_height() - character.capsule.radius;
    let collider_ref = physics.world.colliders.get(collider)?;
    let contact = capsule_contact(
        character,
        &pose,
        collider_ref,
        character.skin * CONTACT_PREDICTION_SKINS,
    )?;
    let at_feet = contact.point1.y <= feet + FOOT_BAND_SKINS * character.skin;
    if contact.normal1.y.abs() < max_slope.cos() || !at_feet {
        return None;
    }
    let body = collider_ref.parent()?;
    let bone = physics.world.bodies.get(body)?.user_data != 0;
    Some(Support {
        drop: hit.time_of_impact,
        normal_y: contact.normal1.y.abs(),
        bone,
        body,
        point: contact.point2,
    })
}

/// Loads the supporting body with the character's weight for one step.
pub(super) fn load_support(
    physics: &mut PhysicsContext,
    character: &Character,
    support: &Support,
    dt: f32,
) {
    physics.shove_body(
        support.body,
        -Vec3::Y * character.mass * character.gravity * dt,
        Some(support.point),
    );
}

fn normalize_with_length(vector: Vec3) -> Option<(Vec3, f32)> {
    let length = vector.length();
    (length > 1.0e-5).then(|| (vector / length, length))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;
    use crate::sim::FIXED_DT;
    use crate::sim::physics::test_support::{
        GROUND_CELL, bake, box_shape, ground_batch, object_group, placement, run_ticks,
        wake_context,
    };

    const SKIN: f32 = 0.05;
    const CRATE_MASS: f32 = 2.0;
    const CRATE_AT: Vec3 = Vec3::new(100.0, 0.5, -100.0);
    const CHARACTER_MASS: f32 = 80.0;
    const CAPSULE_RADIUS: f32 = 0.5;
    const CAPSULE_HALF_CYLINDER: f32 = 1.0;
    const FEET_TO_CENTER: f32 = CAPSULE_HALF_CYLINDER + CAPSULE_RADIUS;

    fn context_with_resting_crate() -> PhysicsContext {
        let mut batch = ground_batch();
        batch.objects.push(object_group(
            "meshes\\crate.glb",
            vec![placement("0A0001:Fallout4.esm", CRATE_AT)],
            true,
            Some(CRATE_MASS),
        ));
        let baked = bake(&batch, &[("meshes\\crate.glb", vec![box_shape(0.5)])]);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 90);
        context
    }

    fn crate_handle(context: &PhysicsContext) -> RigidBodyHandle {
        context
            .world
            .bodies
            .iter()
            .next()
            .map(|(handle, _)| handle)
            .unwrap()
    }

    fn character(capsule: &Capsule, push_force: f32) -> Character<'_> {
        Character {
            capsule,
            mass: CHARACTER_MASS,
            push_force,
            gravity: 68.59,
            skin: SKIN,
        }
    }

    fn capsule() -> Capsule {
        Capsule::new_y(CAPSULE_HALF_CYLINDER, CAPSULE_RADIUS)
    }

    #[test]
    fn approach_limit_is_the_free_distance_before_the_skin() {
        assert!((approach_limit(1.0, SKIN) - 0.95).abs() < 1e-6);
        assert_eq!(approach_limit(SKIN, SKIN), 0.0);
    }

    #[test]
    fn approach_limit_allows_no_approach_inside_the_skin() {
        assert_eq!(approach_limit(0.03, SKIN), 0.0);
        assert_eq!(approach_limit(0.0, SKIN), 0.0);
    }

    #[test]
    fn approach_limit_recovers_penetration_at_a_fraction_of_the_overlap() {
        let step = approach_limit(-0.5, SKIN);
        assert!(step < 0.0 && step > -0.5, "{step}");
    }

    #[test]
    fn a_shove_closes_the_speed_gap_with_the_characters_mass_in_series() {
        let context = context_with_resting_crate();
        let impulse = shove_impulse(
            &context,
            crate_handle(&context),
            CHARACTER_MASS,
            Vec3::X,
            10.0,
            1.0e6,
        )
        .unwrap();
        // closing = 10 * 0.9 = 9; reduced mass 1 / (1/2 + 1/80) = 1 / 0.5125.
        assert!((impulse.x - 17.561).abs() < 0.001, "{impulse}");
        assert_eq!((impulse.y, impulse.z), (0.0, 0.0));
    }

    #[test]
    fn a_shove_impulse_never_exceeds_the_cap() {
        let context = context_with_resting_crate();
        let impulse = shove_impulse(
            &context,
            crate_handle(&context),
            CHARACTER_MASS,
            Vec3::Z,
            1.0e4,
            5.0,
        )
        .unwrap();
        assert!((impulse.length() - 5.0).abs() < 1e-5);
        assert_eq!((impulse.x, impulse.y), (0.0, 0.0));
    }

    #[test]
    fn no_impulse_when_the_body_already_moves_at_least_as_fast() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        context.world.bodies[handle].set_linvel(Vec3::new(10.0, 0.0, 0.0), true);
        assert!(shove_impulse(&context, handle, CHARACTER_MASS, Vec3::X, 9.0, 1.0e6).is_none());
        assert!(shove_impulse(&context, handle, CHARACTER_MASS, Vec3::X, 10.0, 1.0e6).is_none());
        assert!(shove_impulse(&context, handle, CHARACTER_MASS, Vec3::X, 12.0, 1.0e6).is_some());
    }

    #[test]
    fn no_impulse_for_a_body_that_does_not_exist() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        context.world.remove_body(handle);
        assert!(shove_impulse(&context, handle, CHARACTER_MASS, Vec3::X, 10.0, 1.0e6).is_none());
    }

    #[test]
    fn the_capsule_is_stopped_at_the_skin_and_the_crate_is_shoved_within_the_push_force() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        let capsule = capsule();
        let push_force = 100.0;
        let character = character(&capsule, push_force);
        // Standing 0.03 from the crate's -X face (inside the skin), feet on the ground.
        let start = Vec3::new(
            CRATE_AT.x - 0.5 - CAPSULE_RADIUS - 0.03,
            FEET_TO_CENTER,
            CRATE_AT.z,
        );
        let walked = shove_and_slide(
            &mut context,
            &character,
            start,
            Vec3::new(1.0, 0.0, 0.0),
            FIXED_DT,
        );
        assert!(
            walked.x.abs() < 1e-4,
            "the character does not walk into the crate, moved {walked}"
        );
        let speed = context.world.bodies[handle].linvel().x;
        let capped = push_force * FIXED_DT / CRATE_MASS;
        assert!(
            (speed - capped).abs() < 1e-2,
            "speed {speed}, the push-force cap allows {capped}"
        );
    }

    #[test]
    fn a_gentle_walk_into_a_crate_pushes_it_at_the_walking_speed() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        let capsule = capsule();
        let character = character(&capsule, 1.0e6);
        let start = Vec3::new(
            CRATE_AT.x - 0.5 - CAPSULE_RADIUS - 0.03,
            FEET_TO_CENTER,
            CRATE_AT.z,
        );
        let walking_speed = 3.0;
        shove_and_slide(
            &mut context,
            &character,
            start,
            Vec3::new(walking_speed * FIXED_DT, 0.0, 0.0),
            FIXED_DT,
        );
        let speed = context.world.bodies[handle].linvel().x;
        let expected = walking_speed * PUSH_DAMPING / (1.0 + CRATE_MASS / CHARACTER_MASS);
        assert!(
            (speed - expected).abs() < 0.01,
            "speed {speed}, expected {expected}"
        );
    }

    #[test]
    fn a_body_at_foot_level_is_ground_not_an_obstacle() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        let capsule = capsule();
        let character = character(&capsule, 1.0e6);
        // Feet 0.2 above the crate top: only the capsule's bottom cap touches the crate's rim.
        let start = Vec3::new(
            CRATE_AT.x - 0.5 - CAPSULE_RADIUS - 0.03,
            1.0 + FEET_TO_CENTER,
            CRATE_AT.z,
        );
        let motion = Vec3::new(0.1, 0.0, 0.0);
        let walked = shove_and_slide(&mut context, &character, start, motion, FIXED_DT);
        assert!(walked.abs_diff_eq(motion, 1e-5), "{walked}");
        assert!(context.world.bodies[handle].linvel().x.abs() < 1e-3);
    }

    #[test]
    fn support_is_found_for_a_body_just_below_the_feet_and_not_beside_or_far_below() {
        let context = context_with_resting_crate();
        let capsule = capsule();
        let character = character(&capsule, 1.0e6);
        let above = Vec3::new(CRATE_AT.x, 1.0 + FEET_TO_CENTER + 0.02, CRATE_AT.z);
        let reach = 0.3;
        let slope = 60.0_f32.to_radians();
        let support = support_below(&context, &character, above, reach, slope)
            .expect("standing on the crate");
        assert!(support.drop <= reach && support.drop >= 0.0);
        let beside = above + Vec3::new(3.0, 0.0, 0.0);
        assert!(support_below(&context, &character, beside, reach, slope).is_none());
        let high_up = above + Vec3::Y * 10.0;
        assert!(support_below(&context, &character, high_up, reach, slope).is_none());
    }

    #[test]
    fn standing_on_a_body_loads_it_with_the_characters_weight_for_one_step() {
        let mut context = context_with_resting_crate();
        let handle = crate_handle(&context);
        let capsule = capsule();
        let character = character(&capsule, 1.0e6);
        let above = Vec3::new(CRATE_AT.x, 1.0 + FEET_TO_CENTER + 0.02, CRATE_AT.z);
        let support =
            support_below(&context, &character, above, 0.3, 60.0_f32.to_radians()).unwrap();
        let before = context.world.bodies[handle].linvel().y;
        load_support(&mut context, &character, &support, FIXED_DT);
        let gained = context.world.bodies[handle].linvel().y - before;
        let expected = -CHARACTER_MASS * character.gravity * FIXED_DT / CRATE_MASS;
        assert!(
            (gained - expected).abs() < 0.05,
            "gained {gained}, weight impulse gives {expected}"
        );
    }
}
