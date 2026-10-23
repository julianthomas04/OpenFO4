//! Search cost against an independent reference: a plain Dijkstra over (triangle, entry link)
//! states that shares no code with the A* under test.

use std::collections::HashMap;

use glam::Vec3;
use proptest::prelude::*;

use super::support::{MeshBuilder, locate, single_world};
use crate::navmesh::query::NavPoint;
use crate::navmesh::query::search::{Completeness, PathSearch, find_path};
use crate::navmesh::{NavRef, NavWorld, QueryFilter, flags};

/// Cost of walking `refs` from `start` to `goal` under the documented model: inside each triangle
/// the distance from the entry point to the crossing point times the triangle's multiplier, a
/// ledge jump at the landing triangle's multiplier, and the last leg to the goal.
pub fn corridor_cost(
    world: &NavWorld,
    filter: &QueryFilter,
    refs: &[NavRef],
    start: Vec3,
    goal: Vec3,
) -> f32 {
    let multiplier = |nav_ref: NavRef| {
        filter
            .traversal_multiplier(world.mesh(nav_ref).unwrap(), nav_ref.tri())
            .unwrap()
    };
    let mut entry = start;
    let mut cost = 0.0;
    for pair in refs.windows(2) {
        let link = world.link_between(pair[0], pair[1]).unwrap();
        cost += entry.distance(link.crossing()) * multiplier(pair[0]);
        entry = match link.landing {
            Some(landing) => {
                cost += link.crossing().distance(landing) * multiplier(pair[1]);
                landing
            }
            None => link.crossing(),
        };
    }
    if refs.len() > 1 {
        cost += entry.distance(goal) * multiplier(*refs.last().unwrap());
    }
    cost
}

type StateKey = (NavRef, Option<(NavRef, usize)>);

struct State {
    key: StateKey,
    entry: Vec3,
    cost: f32,
    settled: bool,
}

pub fn reference_optimum(
    world: &NavWorld,
    filter: &QueryFilter,
    start: NavPoint,
    goal: NavPoint,
) -> Option<f32> {
    if start.nav_ref == goal.nav_ref {
        return Some(0.0);
    }
    let multiplier =
        |nav_ref: NavRef| filter.traversal_multiplier(world.mesh(nav_ref)?, nav_ref.tri());
    let first_key = (start.nav_ref, None);
    let mut states = vec![State {
        key: first_key,
        entry: start.pos,
        cost: 0.0,
        settled: false,
    }];
    let mut index: HashMap<StateKey, usize> = HashMap::from([(first_key, 0)]);
    let mut best: Option<f32> = None;
    loop {
        let next = states
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.settled)
            .min_by(|a, b| a.1.cost.total_cmp(&b.1.cost))
            .map(|(i, _)| i);
        let Some(current) = next else {
            return best;
        };
        states[current].settled = true;
        let (nav_ref, entry, cost) = (
            states[current].key.0,
            states[current].entry,
            states[current].cost,
        );
        if nav_ref == goal.nav_ref {
            let total = cost + entry.distance(goal.pos) * multiplier(nav_ref).unwrap();
            best = Some(best.map_or(total, |b: f32| b.min(total)));
            continue;
        }
        let here = multiplier(nav_ref).unwrap();
        for (link_index, link) in world.links(nav_ref).enumerate() {
            let Some(there) = multiplier(link.target) else {
                continue;
            };
            if !filter.allows_link(link.link_type) {
                continue;
            }
            let mut next_cost = cost + entry.distance(link.crossing()) * here;
            let next_entry = match link.landing {
                Some(landing) => {
                    next_cost += link.crossing().distance(landing) * there;
                    landing
                }
                None => link.crossing(),
            };
            let key = (link.target, Some((nav_ref, link_index)));
            match index.get(&key) {
                Some(&existing) if states[existing].cost <= next_cost => {}
                Some(&existing) => {
                    states[existing] = State {
                        key,
                        entry: next_entry,
                        cost: next_cost,
                        settled: false,
                    };
                }
                None => {
                    index.insert(key, states.len());
                    states.push(State {
                        key,
                        entry: next_entry,
                        cost: next_cost,
                        settled: false,
                    });
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Case {
    cols: usize,
    rows: usize,
    kept: Vec<bool>,
    tri_flags: Vec<u16>,
    water: f32,
    preferred: f32,
    start: usize,
    goal: usize,
}

fn case() -> impl Strategy<Value = Case> {
    (2usize..5, 1usize..4).prop_flat_map(|(cols, rows)| {
        let cells = cols * rows;
        (
            proptest::collection::vec(proptest::bool::weighted(0.85), cells),
            proptest::collection::vec(
                prop_oneof![Just(0u16), Just(flags::WATER), Just(flags::PREFERRED)],
                cells * 2,
            ),
            0.05f32..6.0,
            0.05f32..1.0,
            0usize..64,
            0usize..64,
        )
            .prop_map(
                move |(mut kept, tri_flags, water, preferred, start, goal)| {
                    kept[0] = true;
                    Case {
                        cols,
                        rows,
                        kept,
                        tri_flags,
                        water,
                        preferred,
                        start,
                        goal,
                    }
                },
            )
    })
}

fn build(case: &Case) -> NavWorld {
    let kept = case.kept.clone();
    let cols = case.cols;
    let mut builder = MeshBuilder::new(1).grid(
        (0.0, 0.0),
        case.cols,
        case.rows,
        1.0,
        |_, _| 0.0,
        move |col, row| kept[row * cols + col],
    );
    let tri_count = case.kept.iter().filter(|k| **k).count() * 2;
    for tri in 0..tri_count {
        builder = builder.flag(tri as u16, case.tri_flags[tri]);
    }
    single_world(builder)
}

fn filter_of(case: &Case) -> QueryFilter {
    QueryFilter::default()
        .with_multiplier(flags::WATER, case.water)
        .unwrap()
        .with_multiplier(flags::PREFERRED, case.preferred)
        .unwrap()
}

fn point_in(world: &NavWorld, tri: usize, count: usize) -> NavPoint {
    let nav_ref = world.nav_ref(1, (tri % count) as u16).unwrap();
    let [a, b, c] = world
        .mesh(nav_ref)
        .unwrap()
        .triangle_vertices(nav_ref.tri())
        .unwrap();
    NavPoint {
        nav_ref,
        pos: a * 0.5 + b * 0.3 + c * 0.2,
    }
}

proptest! {
    #[test]
    fn search_cost_equals_the_reference_dijkstra_optimum(case in case()) {
        let world = build(&case);
        let filter = filter_of(&case);
        let count = world.stats().triangles;
        let (start, goal) = (point_in(&world, case.start, count), point_in(&world, case.goal, count));
        let result = find_path(&world, &mut PathSearch::new(100_000), start, goal, &filter).unwrap();
        if let Some(best) = reference_optimum(&world, &filter, start, goal) {
            prop_assert_eq!(result.completeness, Completeness::Complete);
            let cost = corridor_cost(&world, &filter, &result.refs, start.pos, goal.pos);
            prop_assert!(
                (cost - best).abs() <= 1e-3 * (1.0 + best),
                "search cost {} vs optimum {}", cost, best
            );
        } else {
            prop_assert_eq!(result.completeness, Completeness::Partial);
        }
    }
}

/// A lane of costly terrain runs straight from start to goal with plain rows beside it. A
/// heuristic that overestimates the remaining cost makes the search commit to the lane before the
/// slightly cheaper detour along the plain rows is looked at.
#[test]
fn a_costly_lane_does_not_trick_the_search_into_a_dearer_route() {
    for multiplier in [1.1f32, 1.2, 1.3, 1.5] {
        let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 8, 3);
        for col in 0..8 {
            let first = ((8 + col) * 2) as u16;
            builder = builder
                .flag(first, flags::WATER)
                .flag(first + 1, flags::WATER);
        }
        let world = single_world(builder);
        let filter = QueryFilter::default()
            .with_multiplier(flags::WATER, multiplier)
            .unwrap();
        let (start, goal) = (
            locate(&world, Vec3::new(0.3, 0.0, 1.5)),
            locate(&world, Vec3::new(7.7, 0.0, 1.5)),
        );
        let result = find_path(&world, &mut PathSearch::new(4096), start, goal, &filter).unwrap();
        let cost = corridor_cost(&world, &filter, &result.refs, start.pos, goal.pos);
        let best = reference_optimum(&world, &filter, start, goal).unwrap();
        assert!(
            (cost - best).abs() <= 1e-3 * (1.0 + best),
            "multiplier {multiplier}: search cost {cost} vs optimum {best}"
        );
    }
}
