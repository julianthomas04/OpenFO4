use glam::Vec3;
use proptest::prelude::*;

use super::support::{
    MeshBuilder, locate, nearest as find_nearest, path_length, single_world, stacked_floors,
};
use crate::navmesh::geometry::{point_in_triangle, xz};
use crate::navmesh::query::funnel::string_pull;
use crate::navmesh::query::nearest::NearestParams;
use crate::navmesh::query::raycast::{DEFAULT_MAX_STEPS, raycast};
use crate::navmesh::query::search::{Completeness, PathSearch, find_path};
use crate::navmesh::{NavRef, NavWorld, QueryFilter};

#[derive(Debug, Clone)]
struct GridCase {
    cols: usize,
    rows: usize,
    kept: Vec<bool>,
    game_space: bool,
}

impl GridCase {
    fn is_kept(&self, col: i64, row: i64) -> bool {
        (0..self.cols as i64).contains(&col)
            && (0..self.rows as i64).contains(&row)
            && self.kept[row as usize * self.cols + col as usize]
    }

    fn sign(&self) -> f32 {
        if self.game_space { -1.0 } else { 1.0 }
    }

    /// Plane point (X, B) to the engine position of this fixture.
    fn engine(&self, x: f32, b: f32) -> Vec3 {
        Vec3::new(x, 0.0, b * self.sign())
    }

    fn world(&self) -> NavWorld {
        let case = self.clone();
        let mut builder = MeshBuilder::new(1);
        if self.game_space {
            builder = builder.game_space();
        }
        single_world(builder.grid(
            (0.0, 0.0),
            self.cols,
            self.rows,
            1.0,
            |_, _| 0.0,
            move |col, row| case.is_kept(col as i64, row as i64),
        ))
    }

    /// True when the plane point lies in (or within `slack` of) a kept cell.
    fn covers(&self, x: f32, b: f32, slack: f32) -> bool {
        let (c0, c1) = ((x - slack).floor() as i64, (x + slack).floor() as i64);
        let (r0, r1) = ((b - slack).floor() as i64, (b + slack).floor() as i64);
        (r0..=r1).any(|r| (c0..=c1).any(|c| self.is_kept(c, r)))
    }
}

fn grid_case() -> impl Strategy<Value = GridCase> {
    (2usize..7, 2usize..7, any::<bool>()).prop_flat_map(|(cols, rows, game_space)| {
        proptest::collection::vec(proptest::bool::weighted(0.8), cols * rows).prop_map(
            move |mut kept| {
                kept[0] = true;
                GridCase {
                    cols,
                    rows,
                    kept,
                    game_space,
                }
            },
        )
    })
}

fn kept_cell_point(case: &GridCase, pick: usize, u: f32, v: f32) -> (f32, f32) {
    let cells: Vec<usize> = (0..case.kept.len()).filter(|&i| case.kept[i]).collect();
    let cell = cells[pick % cells.len()];
    ((cell % case.cols) as f32 + u, (cell / case.cols) as f32 + v)
}

const FRACTION: std::ops::Range<f32> = 0.03..0.97;

proptest! {
    #[test]
    fn nearest_agrees_with_the_cell_mask(
        case in grid_case(),
        x in -0.5f32..7.5,
        b in -0.5f32..7.5,
    ) {
        let world = case.world();
        let params = NearestParams { below: 1.0, above: 1.0, step_tolerance: 0.1, fallback_radius: 0.0 };
        let found = find_nearest(&world, case.engine(x, b), params, &QueryFilter::default());
        prop_assert_eq!(found.is_some(), case.is_kept(x.floor() as i64, b.floor() as i64));
        if let Some(found) = found {
            prop_assert!(found.inside);
            prop_assert!(found.point.y.abs() < 1e-4);
        }
    }

    #[test]
    fn nearest_agrees_with_brute_force_on_stacked_surfaces(
        x in 0.01f32..7.99,
        z in 0.01f32..1.99,
        y in -2.0f32..7.0,
        below in 0.0f32..6.0,
        above in 0.0f32..6.0,
        step in 0.0f32..3.0,
    ) {
        let world = single_world(stacked_floors(1));
        let params = NearestParams { below, above, step_tolerance: step, fallback_radius: 0.0 };
        let found = find_nearest(&world, Vec3::new(x, y, z), params, &QueryFilter::default()).map(|r| r.point.y);

        let mesh = world.resident().next().unwrap().mesh;
        let heights: Vec<f32> = (0..mesh.triangle_count() as u16)
            .filter(|&t| point_in_triangle(xz(Vec3::new(x, 0.0, z)), mesh.triangle_vertices(t).unwrap().map(xz)))
            .filter_map(|t| mesh.height_at(t, x, z))
            .filter(|h| (y - below..=y + above).contains(h))
            .collect();
        let at_or_below = heights.iter().copied().filter(|h| *h <= y + step).fold(None, |a: Option<f32>, h| Some(a.map_or(h, |a| a.max(h))));
        let above_step = heights.iter().copied().filter(|h| *h > y + step).fold(None, |a: Option<f32>, h| Some(a.map_or(h, |a| a.min(h))));
        let expected = at_or_below.or(above_step);
        prop_assert_eq!(found.is_some(), expected.is_some());
        if let (Some(found), Some(expected)) = (found, expected) {
            prop_assert!((found - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn funnel_output_is_short_valid_and_symmetric(
        case in grid_case(),
        pick_a in 0usize..64, pick_b in 0usize..64,
        (ua, va, ub, vb) in (FRACTION, FRACTION, FRACTION, FRACTION),
    ) {
        let world = case.world();
        let (ax, ab) = kept_cell_point(&case, pick_a, ua, va);
        let (bx, bb) = kept_cell_point(&case, pick_b, ub, vb);
        let (start, goal) = (locate(&world, case.engine(ax, ab)), locate(&world, case.engine(bx, bb)));
        let found = find_path(&world, &mut PathSearch::new(2048), start, goal, &QueryFilter::default()).unwrap();
        if found.completeness == Completeness::Complete {
            let points = string_pull(&world, &found.refs, start.pos, goal.pos, 0.0).unwrap();
            let length = path_length(&points);

            prop_assert!(length >= start.pos.distance(goal.pos) - 1e-3);
            prop_assert!(length <= midpoint_length(&world, &found.refs, start.pos, goal.pos) + 1e-3);
            for pair in points.windows(2) {
                for step in 0..=10 {
                    let p = pair[0].lerp(pair[1], step as f32 / 10.0);
                    prop_assert!(case.covers(p.x, p.z * case.sign(), 1e-3), "{p:?} left the mesh");
                }
            }

            let reversed: Vec<NavRef> = found.refs.iter().rev().copied().collect();
            let back = string_pull(&world, &reversed, goal.pos, start.pos, 0.0).unwrap();
            prop_assert!((path_length(&back) - length).abs() < 1e-3);
        }
    }

    #[test]
    fn reachability_is_symmetric_without_one_way_links(
        case in grid_case(),
        pick_a in 0usize..64, pick_b in 0usize..64,
    ) {
        let world = case.world();
        let (ax, ab) = kept_cell_point(&case, pick_a, 0.4, 0.6);
        let (bx, bb) = kept_cell_point(&case, pick_b, 0.7, 0.3);
        let (a, b) = (locate(&world, case.engine(ax, ab)), locate(&world, case.engine(bx, bb)));
        let mut search = PathSearch::new(2048);
        let filter = QueryFilter::default();
        let forward = find_path(&world, &mut search, a, b, &filter).unwrap().completeness;
        let backward = find_path(&world, &mut search, b, a, &filter).unwrap().completeness;
        prop_assert_eq!(forward, backward);
    }

    #[test]
    fn raycast_agrees_with_sampling_the_cell_mask(
        case in grid_case(),
        pick in 0usize..64,
        (u, v) in (FRACTION, FRACTION),
        (ex, eb) in (-1.0f32..8.0, -1.0f32..8.0),
    ) {
        const SAMPLES: usize = 4000;
        let world = case.world();
        let (sx, sb) = kept_cell_point(&case, pick, u, v);
        // A ray through a cell corner is a tie between the edges meeting there; the walk may go
        // either way, so those inputs say nothing about correctness.
        prop_assume!(!passes_near_a_lattice_corner((sx, sb), (ex, eb)));
        let start = locate(&world, case.engine(sx, sb));
        let result = raycast(&world, start, case.engine(ex, eb), &QueryFilter::default(), DEFAULT_MAX_STEPS).unwrap();

        let first_miss = (1..=SAMPLES).find(|i| {
            let t = *i as f32 / SAMPLES as f32;
            let (x, b) = (sx + (ex - sx) * t, sb + (eb - sb) * t);
            !case.is_kept(x.floor() as i64, b.floor() as i64)
        });
        match (result.hit, first_miss) {
            (None, None) => {}
            (Some(hit), Some(i)) => {
                let t = i as f32 / SAMPLES as f32;
                prop_assert!(hit.t <= t + 1e-3 && hit.t >= t - 2.0 / SAMPLES as f32 - 1e-3, "hit {} vs sampled {}", hit.t, t);
            }
            (hit, miss) => prop_assert!(false, "raycast {hit:?} vs sampling {miss:?}"),
        }
    }
}

/// Whether the segment passes within 1e-2 of an integer lattice point.
fn passes_near_a_lattice_corner(from: (f32, f32), to: (f32, f32)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length_sq = dx * dx + dy * dy;
    (-1..=8).any(|i| {
        (-1..=8).any(|j| {
            let (px, py) = (i as f32 - from.0, j as f32 - from.1);
            let along = if length_sq > 0.0 {
                ((px * dx + py * dy) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (px - dx * along).hypot(py - dy * along) < 1e-2
        })
    })
}

fn midpoint_length(world: &NavWorld, refs: &[NavRef], start: Vec3, goal: Vec3) -> f32 {
    let mut points = vec![start];
    for pair in refs.windows(2) {
        let link = world.link_between(pair[0], pair[1]).unwrap();
        let (a, b) = world
            .mesh(pair[0])
            .unwrap()
            .edge_vertices(pair[0].tri(), link.edge)
            .unwrap();
        points.push(a.midpoint(b));
    }
    points.push(goal);
    path_length(&points)
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Load(u32),
    Unload(u32),
}

const RING: u32 = 4;

fn ring_mesh(index: u32) -> MeshBuilder {
    let (next, prev) = ((index + 1) % RING, (index + RING - 1) % RING);
    MeshBuilder::new(index + 1)
        .flat_grid((index as f32 * 2.0, 0.0), 1, 1)
        .link((0, 1), 0, next + 1, (1, 2))
        .link((1, 2), 0, prev + 1, (0, 1))
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(
        (0..RING, any::<bool>())
            .prop_map(|(i, load)| if load { Op::Load(i) } else { Op::Unload(i) }),
        1..40,
    )
}

proptest! {
    #[test]
    fn random_load_unload_order_never_yields_stale_or_dangling_links(ops in ops()) {
        let mut world = NavWorld::new();
        let mut loaded = [false; RING as usize];
        let mut remembered: Vec<(u32, NavRef)> = Vec::new();
        for op in ops {
            match op {
                Op::Load(i) if !loaded[i as usize] => {
                    world.add(ring_mesh(i).mesh()).unwrap();
                    loaded[i as usize] = true;
                    remembered.extend((0..2).map(|t| (i, world.nav_ref(i + 1, t).unwrap())));
                }
                Op::Unload(i) if loaded[i as usize] => {
                    world.remove(i + 1).unwrap();
                    loaded[i as usize] = false;
                }
                _ => {}
            }
            for &(mesh, nav_ref) in &remembered {
                let current = world.nav_ref(mesh + 1, nav_ref.tri());
                prop_assert_eq!(world.is_valid(nav_ref), current == Some(nav_ref));
            }
            let mut expected_pending = 0;
            for i in (0..RING).filter(|&i| loaded[i as usize]) {
                let (next, prev) = ((i + 1) % RING, (i + RING - 1) % RING);
                for (tri, target) in [(0u16, next), (1, prev)] {
                    let from = world.nav_ref(i + 1, tri).unwrap();
                    let externals: Vec<_> = world.links(from).filter(|l| l.link_type.is_some()).collect();
                    if loaded[target as usize] {
                        prop_assert_eq!(externals.len(), 1);
                        prop_assert!(world.is_valid(externals[0].target));
                        prop_assert_eq!(world.mesh(externals[0].target).unwrap().form_id(), target + 1);
                    } else {
                        prop_assert!(externals.is_empty());
                        expected_pending += 1;
                    }
                }
            }
            prop_assert_eq!(world.pending_link_count(), expected_pending);
        }
    }
}
