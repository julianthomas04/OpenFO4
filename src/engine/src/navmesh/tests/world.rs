use super::support::{MeshBuilder, linked_pair, nav_ref};
use crate::navmesh::{LinkType, NavMesh, NavStats, NavWorld, WorldError};

const A: u32 = 0xA;
const B: u32 = 0xB;

fn external_targets(world: &NavWorld, form_id: u32, tri: u16) -> Vec<(u32, u16)> {
    world
        .links(nav_ref(world, form_id, tri))
        .filter(|l| l.link_type.is_some())
        .map(|l| (world.mesh(l.target).unwrap().form_id(), l.target.tri()))
        .collect()
}

fn load(world: &mut NavWorld, builder: MeshBuilder) -> crate::navmesh::LoadReport {
    world.add(builder.mesh()).unwrap()
}

#[test]
fn duplicate_load_and_unknown_unload_are_errors() {
    let mut world = NavWorld::new();
    load(&mut world, MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1));
    let again = world.add(MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1).mesh());
    assert_eq!(again.unwrap_err(), WorldError::AlreadyLoaded(A));
    assert_eq!(world.remove(B).unwrap_err(), WorldError::NotLoaded(B));
}

#[test]
fn links_resolve_in_either_load_order() {
    for order in [[A, B], [B, A]] {
        let mut world = NavWorld::new();
        for form in order {
            let (a, b) = linked_pair(A, B);
            load(&mut world, if form == A { a } else { b });
        }
        assert_eq!(external_targets(&world, A, 2), vec![(B, 1)]);
        assert_eq!(external_targets(&world, B, 1), vec![(A, 2)]);
        assert_eq!(world.pending_link_count(), 0);
    }
}

#[test]
fn link_types_survive_resolution() {
    let mut world = NavWorld::new();
    let a = MeshBuilder::new(A)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 1), 3, B, (0, 0));
    load(&mut world, a);
    load(&mut world, MeshBuilder::new(B).flat_grid((1.0, 0.0), 1, 1));
    let link = world.links(nav_ref(&world, A, 0)).last().unwrap();
    assert_eq!(link.link_type, Some(LinkType::EnableDisablePortal));
    assert_eq!(link.edge, 1);
}

#[test]
fn unload_defers_links_and_reload_restores_them() {
    let mut world = NavWorld::new();
    let (a, b) = linked_pair(A, B);
    load(&mut world, a);
    load(&mut world, b);
    let stale = nav_ref(&world, B, 1);

    let report = world.remove(B).unwrap();
    assert_eq!(report.links_deferred, 1);
    assert_eq!(world.pending_link_count(), 1);
    assert!(external_targets(&world, A, 2).is_empty());
    assert!(!world.is_valid(stale));

    let (_, b) = linked_pair(A, B);
    let report = load(&mut world, b);
    assert_eq!(report.linked, 2);
    assert_eq!(external_targets(&world, A, 2), vec![(B, 1)]);
    assert!(!world.is_valid(stale));
    assert!(world.is_valid(nav_ref(&world, B, 1)));
}

#[test]
fn dangling_link_stays_pending_and_is_not_an_error() {
    let mut world = NavWorld::new();
    let report = load(
        &mut world,
        MeshBuilder::new(A)
            .flat_grid((0.0, 0.0), 1, 1)
            .link((0, 1), 0, 0xDEAD, (0, 0)),
    );
    assert_eq!((report.linked, report.deferred), (0, 1));
    assert!(report.rejected.is_empty());
    assert_eq!(world.pending_link_count(), 1);
    world.remove(A).unwrap();
    assert_eq!(world.pending_link_count(), 0);
}

#[test]
fn link_to_missing_target_triangle_is_reported_as_rejected() {
    let mut world = NavWorld::new();
    load(&mut world, MeshBuilder::new(B).flat_grid((1.0, 0.0), 1, 1));
    let report = load(
        &mut world,
        MeshBuilder::new(A)
            .flat_grid((0.0, 0.0), 1, 1)
            .link((0, 1), 0, B, (77, 0)),
    );
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.rejected[0].source_form_id, A);
    assert!(external_targets(&world, A, 0).is_empty());
}

#[test]
fn reused_slot_does_not_revive_stale_refs() {
    let mut world = NavWorld::new();
    load(&mut world, MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1));
    let stale = nav_ref(&world, A, 0);
    world.remove(A).unwrap();
    load(&mut world, MeshBuilder::new(B).flat_grid((0.0, 0.0), 1, 1));
    let fresh = nav_ref(&world, B, 0);
    assert_eq!(stale.slot(), fresh.slot());
    assert!(!world.is_valid(stale));
    assert!(world.is_valid(fresh));
    assert!(world.mesh(stale).is_none());
    assert_eq!(world.links(stale).count(), 0);
}

#[test]
fn out_of_range_triangle_is_invalid() {
    let mut world = NavWorld::new();
    load(&mut world, MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1));
    assert!(world.nav_ref(A, 2).is_none());
    let valid = nav_ref(&world, A, 1);
    let beyond = crate::navmesh::NavRef::new(valid.slot(), valid.salt(), 2);
    assert!(!world.is_valid(beyond));
}

#[test]
fn salt_is_never_zero_across_wraparound() {
    let template: NavMesh = MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1).mesh();
    let mut world = NavWorld::new();
    for _ in 0..=u32::from(u16::MAX) {
        let report = world.add(template.clone()).unwrap();
        assert_ne!(report.salt, 0);
        world.remove(A).unwrap();
    }
}

#[test]
fn resolved_link_count_follows_load_and_unload() {
    let mut world = NavWorld::new();
    let (a, b) = linked_pair(A, B);
    load(&mut world, a);
    assert_eq!(world.resolved_link_count(), 0);
    load(&mut world, b);
    assert_eq!(world.resolved_link_count(), 2);
    world.remove(A).unwrap();
    assert_eq!(world.resolved_link_count(), 0);
}

#[test]
fn generation_changes_on_every_membership_change() {
    let mut world = NavWorld::new();
    let initial = world.generation();
    load(&mut world, MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1));
    let after_add = world.generation();
    world.remove(A).unwrap();
    let after_remove = world.generation();
    world.clear();
    let after_clear = world.generation();
    assert!(initial < after_add && after_add < after_remove && after_remove < after_clear);
}

#[test]
fn clear_empties_the_world_and_stales_references() {
    let mut world = NavWorld::new();
    let (a, b) = linked_pair(A, B);
    load(&mut world, a);
    load(&mut world, b);
    let stale = nav_ref(&world, A, 0);

    world.clear();

    assert_eq!(world.resident_count(), 0);
    assert_eq!(world.pending_link_count(), 0);
    assert_eq!(world.resolved_link_count(), 0);
    assert!(!world.is_valid(stale));
    load(&mut world, MeshBuilder::new(A).flat_grid((0.0, 0.0), 1, 1));
    assert!(!world.is_valid(stale));
    assert!(world.is_valid(nav_ref(&world, A, 0)));
}

#[test]
fn stats_total_meshes_triangles_and_links() {
    let mut world = NavWorld::new();
    let (a, b) = linked_pair(A, B);
    load(&mut world, a);
    load(
        &mut world,
        MeshBuilder::new(0xC)
            .flat_grid((10.0, 0.0), 1, 1)
            .link((0, 1), 0, 0xDEAD, (0, 0)),
    );
    load(&mut world, b);
    assert_eq!(
        world.stats(),
        NavStats {
            meshes: 3,
            triangles: 4 + 4 + 2,
            resolved_links: 2,
            pending_links: 1,
            ..NavStats::default()
        }
    );
}
