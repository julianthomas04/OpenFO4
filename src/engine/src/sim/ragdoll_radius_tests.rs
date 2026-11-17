//! First contacts of the installed ragdolls in their authored poses with the converted collision
//! (round hulls grown by Havok's convex radius), compared with the same pairs measured between the
//! bare hulls. Ignored: needs a converted installation (`OPENFO4_INSTALL_DIR`).

use std::collections::{BTreeMap, HashSet};

use rapier3d::parry::query::contact;
use rapier3d::prelude::*;

use super::{EMBEDDED_DEPTH, MANIFOLD_EMBEDDED};
use crate::rendering::world_streamer::real_install::interior_collision;
use crate::sim::CellGrid;
use crate::sim::physics::PhysicsContext;

/// Interiors with the most authored ragdoll poses (XRGD) in the installed database:
/// `FallonsDepartmentStore01`, `Vault75`, `TheBigDig01`, `MiltonGeneral01`.
const CELLS: [&str; 4] = [
    "03D7DC:Fallout4.esm",
    "0C01A7:Fallout4.esm",
    "04E428:Fallout4.esm",
    "03430D:Fallout4.esm",
];
const GRID: CellGrid = (0, 0);
/// Each reconcile spawns a bounded number of ragdolls; this many cover the live-ragdoll cap.
const RECONCILE_ROUNDS: usize = 16;
/// Half-width of the band around `EMBEDDED_DEPTH` in which float noise could flip a class.
const BOUNDARY_BAND: f32 = 0.02;
/// How far apart two bare hulls may be and still report their distance.
const BARE_PREDICTION: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Touching,
    Embedded,
}

/// What a ragdoll bone touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Partner {
    /// A bone of another ragdoll.
    Bone,
    /// A collider with a round hull (grown by the convex radius).
    RoundHull,
    /// A collider without one (triangle meshes, boxes, capsules).
    Unrounded,
}

/// One bone contact pair after the first step.
struct PairContact {
    partner: Partner,
    grown_depth: f32,
    grown_class: Class,
    /// Penetration of the bare hulls, `None` beyond `BARE_PREDICTION` apart.
    bare_depth: Option<f32>,
    /// Sum of both colliders' convex radii.
    radii: f32,
    bone_radius: f32,
}

impl PairContact {
    /// The class `RagdollHooks` gives a frozen (motionless) bone at this bare depth.
    fn bare_class(&self) -> Class {
        match self.bare_depth {
            Some(depth) if depth > EMBEDDED_DEPTH => Class::Embedded,
            _ => Class::Touching,
        }
    }
}

struct Scene {
    ragdolls: usize,
    bones: usize,
    contacts: Vec<PairContact>,
    unsupported: usize,
}

/// `shape` with every round hull replaced by its bare hull, or `None` when it has none.
fn bare(shape: &SharedShape) -> Option<SharedShape> {
    if let Some(round) = shape.as_round_convex_polyhedron() {
        return Some(SharedShape::new(round.inner_shape.clone()));
    }
    let leaves = shape.as_compound()?.shapes();
    let stripped: Vec<Option<SharedShape>> = leaves.iter().map(|(_, leaf)| bare(leaf)).collect();
    if stripped.iter().all(Option::is_none) {
        return None;
    }
    let parts = leaves
        .iter()
        .zip(stripped)
        .map(|((pose, leaf), stripped)| (*pose, stripped.unwrap_or_else(|| leaf.clone())))
        .collect();
    Some(SharedShape::compound(parts))
}

/// The convex radius `shape` is grown by: its own, or the largest of its leaves'.
fn round_radius(shape: &SharedShape) -> f32 {
    if let Some(round) = shape.as_round_convex_polyhedron() {
        return round.border_radius;
    }
    shape.as_compound().map_or(0.0, |compound| {
        compound
            .shapes()
            .iter()
            .map(|(_, leaf)| round_radius(leaf))
            .fold(0.0, f32::max)
    })
}

fn is_bone(collider: &Collider) -> bool {
    collider
        .active_hooks()
        .contains(ActiveHooks::MODIFY_SOLVER_CONTACTS)
}

fn partner_of(collider: &Collider) -> Partner {
    if is_bone(collider) {
        Partner::Bone
    } else if bare(collider.shared_shape()).is_some() {
        Partner::RoundHull
    } else {
        Partner::Unrounded
    }
}

/// Spawns every ragdoll of the cell frozen in its authored pose, steps once and reads the class
/// `RagdollHooks` gave each bone contact pair, next to the depth of the pair's bare hulls.
fn first_contacts(cell: &str) -> Scene {
    let (batch, baked) = interior_collision(cell);
    let mut context = PhysicsContext::new();
    context.set_interior_mode(true);
    context.add_cell(GRID, &batch, Some(&baked));
    for _ in 0..RECONCILE_ROUNDS {
        context.set_physics_center(GRID, 1);
    }
    context.step();

    let world = &context.world;
    let bone_bodies: Vec<&RigidBody> = world
        .bodies
        .iter()
        .map(|(_, body)| body)
        .filter(|body| {
            !body.is_fixed()
                && body
                    .colliders()
                    .iter()
                    .any(|handle| world.colliders.get(*handle).is_some_and(is_bone))
        })
        .collect();
    let ragdolls = bone_bodies
        .iter()
        .map(|body| body.user_data)
        .collect::<HashSet<_>>()
        .len();
    let mut contacts = Vec::new();
    let mut unsupported = 0;
    for pair in world.narrow_phase.contact_pairs() {
        let (Some(first), Some(second)) = (
            world.colliders.get(pair.collider1),
            world.colliders.get(pair.collider2),
        ) else {
            continue;
        };
        let (bone, other) = match (is_bone(first), is_bone(second)) {
            (true, _) => (first, second),
            (false, true) => (second, first),
            (false, false) => continue,
        };
        let active: Vec<&ContactManifold> = pair
            .manifolds
            .iter()
            .filter(|manifold| !manifold.data.solver_contacts.is_empty())
            .collect();
        // The manifold points, not the solver contacts: `RagdollHooks` clamps those.
        let Some(grown_depth) = active
            .iter()
            .flat_map(|manifold| &manifold.points)
            .map(|point| -point.dist)
            .reduce(f32::max)
        else {
            continue;
        };
        let grown_class = if active
            .iter()
            .any(|manifold| manifold.data.user_data == MANIFOLD_EMBEDDED)
        {
            Class::Embedded
        } else {
            Class::Touching
        };
        let bare_bone = bare(bone.shared_shape()).unwrap_or_else(|| bone.shared_shape().clone());
        let bare_other = bare(other.shared_shape()).unwrap_or_else(|| other.shared_shape().clone());
        let Ok(bare_contact) = contact(
            bone.position(),
            &*bare_bone,
            other.position(),
            &*bare_other,
            BARE_PREDICTION,
        ) else {
            unsupported += 1;
            continue;
        };
        contacts.push(PairContact {
            partner: partner_of(other),
            grown_depth,
            grown_class,
            bare_depth: bare_contact.map(|found| -found.dist),
            radii: round_radius(bone.shared_shape()) + round_radius(other.shared_shape()),
            bone_radius: round_radius(bone.shared_shape()),
        });
    }
    Scene {
        ragdolls,
        bones: bone_bodies.len(),
        contacts,
        unsupported,
    }
}

/// Contact depths bucketed by edges in engine units (penetration positive).
fn histogram(depths: impl Iterator<Item = Option<f32>>) -> Vec<(String, usize)> {
    const EDGES: [f32; 7] = [-0.14, -0.07, -0.02, 0.02, 0.05, 0.09, 0.2];
    let mut buckets = vec![0usize; EDGES.len() + 2];
    for depth in depths {
        let index = depth.map_or(0, |depth| {
            1 + EDGES
                .iter()
                .position(|edge| depth < *edge)
                .unwrap_or(EDGES.len())
        });
        buckets[index] += 1;
    }
    buckets
        .into_iter()
        .enumerate()
        .map(|(index, n)| {
            let label = if index == 0 {
                format!("apart>{BARE_PREDICTION}")
            } else {
                let low = EDGES
                    .get(index.wrapping_sub(2))
                    .map_or("-inf".to_owned(), ToString::to_string);
                let high = EDGES
                    .get(index - 1)
                    .map_or("inf".to_owned(), ToString::to_string);
                format!("[{low},{high})")
            };
            (label, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

fn near_boundary(depth: Option<f32>) -> bool {
    depth.is_some_and(|depth| (depth - EMBEDDED_DEPTH).abs() <= BOUNDARY_BAND)
}

#[derive(Default, Debug)]
struct Tally {
    pairs: usize,
    grown_embedded: usize,
    bare_embedded: usize,
    touching_to_embedded: usize,
    embedded_to_touching: usize,
    grown_near_boundary: usize,
    bare_near_boundary: usize,
    /// Pairs whose hook class differs from what their grown depth alone gives (an impact).
    hook_differs_from_depth: usize,
}

fn tally<'a>(contacts: impl Iterator<Item = &'a PairContact>) -> Tally {
    let mut tally = Tally::default();
    for pair in contacts {
        tally.pairs += 1;
        tally.grown_embedded += usize::from(pair.grown_class == Class::Embedded);
        tally.bare_embedded += usize::from(pair.bare_class() == Class::Embedded);
        match (pair.bare_class(), pair.grown_class) {
            (Class::Touching, Class::Embedded) => tally.touching_to_embedded += 1,
            (Class::Embedded, Class::Touching) => tally.embedded_to_touching += 1,
            _ => {}
        }
        tally.grown_near_boundary += usize::from(near_boundary(Some(pair.grown_depth)));
        tally.bare_near_boundary += usize::from(near_boundary(pair.bare_depth));
        let by_depth = if pair.grown_depth > EMBEDDED_DEPTH {
            Class::Embedded
        } else {
            Class::Touching
        };
        tally.hook_differs_from_depth += usize::from(by_depth != pair.grown_class);
    }
    tally
}

/// Where the authored poses rest: the gap between the bare hulls of each pair that is near
/// contact, minus the pair's convex radii (0 = the grown hulls just touch), per partner and for
/// bones at the most common radius (an unscaled placement) apart from larger ones (scaled
/// skeletons).
fn report_rest_gaps(contacts: &[PairContact]) {
    const NEAR_CONTACT: f32 = 0.3;
    const UNSCALED_BONE_RADIUS: f32 = 0.075;
    let mut groups: BTreeMap<(Partner, bool), Vec<f32>> = BTreeMap::new();
    for pair in contacts {
        let Some(gap) = pair.bare_depth.map(|depth| -depth) else {
            continue;
        };
        if gap.abs() < NEAR_CONTACT {
            groups
                .entry((pair.partner, pair.bone_radius > UNSCALED_BONE_RADIUS))
                .or_default()
                .push(gap - pair.radii);
        }
    }
    for ((partner, scaled), mut residuals) in groups {
        residuals.sort_by(f32::total_cmp);
        let quartile = |q: usize| residuals[(residuals.len() - 1) * q / 4];
        let within = |tolerance: f32| residuals.iter().filter(|r| r.abs() <= tolerance).count();
        println!(
            "rest gap minus radii, {partner:?}, scaled bone {scaled}: pairs {} quartiles {:?} within 0.01 {} within 0.02 {}",
            residuals.len(),
            [quartile(1), quartile(2), quartile(3)],
            within(0.01),
            within(0.02),
        );
    }
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
/// Prints how the radius moves the first contacts (counts, depths, flips, rest gaps). Asserts the
/// contract the classification keeps on real poses: a frozen bone's class follows its depth, and
/// the radius never turns a bone embedded between bare hulls into one that is pushed out.
fn authored_ragdoll_bones_keep_their_first_contact_class_with_the_convex_radius() {
    let mut all = Vec::new();
    for cell in CELLS {
        let scene = first_contacts(cell);
        println!(
            "{cell}: ragdolls {} bones {} bone pairs {} unsupported queries {} {:?}",
            scene.ragdolls,
            scene.bones,
            scene.contacts.len(),
            scene.unsupported,
            tally(scene.contacts.iter()),
        );
        all.extend(scene.contacts);
    }
    let mut by_partner: BTreeMap<Partner, Vec<&PairContact>> = BTreeMap::new();
    for pair in &all {
        by_partner.entry(pair.partner).or_default().push(pair);
    }
    for (partner, pairs) in &by_partner {
        println!("{partner:?}: {:?}", tally(pairs.iter().copied()));
        println!(
            "  grown depths {:?}",
            histogram(pairs.iter().map(|pair| Some(pair.grown_depth)))
        );
        println!(
            "  bare depths  {:?}",
            histogram(pairs.iter().map(|pair| pair.bare_depth))
        );
    }
    report_rest_gaps(&all);
    let total = tally(all.iter());
    println!("total {total:?}");
    assert!(
        total.pairs > 0,
        "no ragdoll bone touches anything in {CELLS:?}"
    );
    assert_eq!(total.hook_differs_from_depth, 0, "{total:?}");
    assert_eq!(total.embedded_to_touching, 0, "{total:?}");
}
