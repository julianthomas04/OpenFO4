use super::support::{MeshBuilder, linked_pair};
use crate::navmesh::{NavMesh, NavResidency};

const A: u32 = 0xA;
const B: u32 = 0xB;

fn cell_meshes(form_ids: &[u32]) -> Vec<NavMesh> {
    form_ids
        .iter()
        .enumerate()
        .map(|(index, &form_id)| {
            MeshBuilder::new(form_id)
                .flat_grid((index as f32 * 3.0, 0.0), 1, 1)
                .mesh()
        })
        .collect()
}

#[test]
fn integrating_makes_meshes_resident_and_owns_their_ids() {
    let mut nav = NavResidency::new();
    let cell = nav.integrate(cell_meshes(&[1, 2]));
    assert_eq!(cell.form_ids(), [1, 2]);
    assert_eq!(nav.world().resident_count(), 2);
}

#[test]
fn retiring_a_cell_withdraws_exactly_its_meshes() {
    let mut nav = NavResidency::new();
    let first = nav.integrate(cell_meshes(&[1, 2]));
    let second = nav.integrate(cell_meshes(&[3]));
    nav.retire(&first);
    assert_eq!(nav.world().resident_count(), 1);
    assert!(nav.world().nav_ref(3, 0).is_some());
    assert!(nav.world().nav_ref(1, 0).is_none());
    nav.retire(&second);
    assert_eq!(nav.world().resident_count(), 0);
}

#[test]
fn a_deferred_retire_frees_the_ids_for_the_same_grid_to_come_back() {
    let mut nav = NavResidency::new();
    let old = nav.integrate(cell_meshes(&[1, 2]));
    let stale = nav.world().nav_ref(1, 0).unwrap();

    nav.retire(&old);
    let back = nav.integrate(cell_meshes(&[1, 2]));

    assert_eq!(back.form_ids(), [1, 2], "no id was left behind to collide");
    assert_eq!(nav.world().resident_count(), 2);
    assert!(
        !nav.world().is_valid(stale),
        "references from before the retire stay stale"
    );
}

#[test]
fn reintegrating_before_the_retire_is_refused_and_leaves_the_new_cell_empty() {
    let mut nav = NavResidency::new();
    let old = nav.integrate(cell_meshes(&[1]));
    let new = nav.integrate(cell_meshes(&[1]));
    assert!(new.is_empty(), "the id is still owned by the old cell");
    nav.retire(&new);
    assert_eq!(
        nav.world().resident_count(),
        1,
        "the refused cell owns nothing to withdraw"
    );
    nav.retire(&old);
    assert_eq!(nav.world().resident_count(), 0);
}

#[test]
fn a_duplicate_form_id_is_not_owned_by_the_second_cell() {
    let mut nav = NavResidency::new();
    let first = nav.integrate(cell_meshes(&[1, 2]));
    let second = nav.integrate(cell_meshes(&[2, 3]));
    assert_eq!(second.form_ids(), [3]);
    nav.retire(&second);
    assert!(
        nav.world().nav_ref(2, 0).is_some(),
        "mesh 2 still belongs to the first cell"
    );
    assert!(nav.world().nav_ref(3, 0).is_none());
    nav.retire(&first);
    assert_eq!(nav.world().resident_count(), 0);
}

#[test]
fn teardown_clears_the_world_and_a_late_retire_is_harmless() {
    let mut nav = NavResidency::new();
    let cell = nav.integrate(cell_meshes(&[1, 2]));
    let stale = nav.world().nav_ref(1, 0).unwrap();
    nav.clear();
    assert_eq!(nav.world().resident_count(), 0);
    assert!(!nav.world().is_valid(stale));
    nav.retire(&cell);
    assert_eq!(nav.world().resident_count(), 0);
    let fresh = nav.integrate(cell_meshes(&[1]));
    assert_eq!(fresh.form_ids(), [1]);
}

#[test]
fn links_across_cells_follow_integration_and_retirement() {
    let (a, b) = linked_pair(A, B);
    let mut nav = NavResidency::new();
    let cell_a = nav.integrate(vec![a.mesh()]);
    assert_eq!(nav.world().pending_link_count(), 1);
    let _cell_b = nav.integrate(vec![b.mesh()]);
    assert_eq!(nav.world().pending_link_count(), 0);
    assert_eq!(nav.world().resolved_link_count(), 2);

    nav.retire(&cell_a);
    assert_eq!(nav.world().resolved_link_count(), 0);
    assert_eq!(
        nav.world().pending_link_count(),
        1,
        "B's link waits for A to return"
    );
}

#[test]
fn an_empty_cell_owns_nothing() {
    let mut nav = NavResidency::new();
    let cell = nav.integrate(Vec::new());
    assert!(cell.is_empty());
    nav.retire(&cell);
    assert_eq!(nav.world().resident_count(), 0);
}
