//! Every navigation mesh of a converted installation: decode, build, link into one `NavWorld`,
//! report the totals and run queries across real cell borders.
//!
//! Ignored by default (needs an installation). Run with
//! `cargo test --test navmesh_real_data -- --ignored --nocapture`; the database is
//! `OPENFO4_DB` when set, else the `data/data.db` of the installation in `OPENFO4_INSTALL_DIR`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use glam::Vec3;
use openfo4::assets::db::DbClient;
use openfo4::assets::layout::data_dir;
use openfo4::assets::{AssetRootEnv, resolve_asset_root};
use openfo4::navmesh::query::NavPoint;
use openfo4::navmesh::query::funnel::string_pull;
use openfo4::navmesh::query::nearest::{NearestParams, find_nearest};
use openfo4::navmesh::query::raycast::{DEFAULT_MAX_STEPS, RaycastError, raycast};
use openfo4::navmesh::query::search::{Completeness, PathSearch, find_path};
use openfo4::navmesh::{
    BuildError, LinkType, NavMesh, NavRef, NavWorld, QueryFilter, WorldError, build_meshes,
};

const SAMPLED_LINKS: usize = 8;
const SEARCH_NODE_CAP: usize = 16_384;
const LONG_ROUTE_NODE_CAP: usize = 262_144;
const LONG_ROUTE_MIN_MESH_HOPS: u32 = 10;
const LONG_ROUTE_MIN_TRIANGLES: usize = 10;
const LONG_ROUTES_WANTED: usize = 6;
const LONG_ROUTE_CANDIDATES_TRIED: usize = 80;
/// Funnel corners keep this far (engine units) from portal endpoints, so straight segments stay
/// clear of wall vertices; with a zero radius corners sit on them and a few segments graze a wall
/// (up to 12 units short of their end in the real data).
const AGENT_RADIUS: f32 = 8.0;
/// A funnel segment may end this far short of its target by float noise (the real data stays
/// under 1e-3); a longer shortfall means the straight segment really crosses a wall.
const GRAZE_TOLERANCE: f32 = 0.05;
const NEAREST: NearestParams = NearestParams {
    below: 1.0,
    above: 1.0,
    step_tolerance: 0.3,
    fallback_radius: 0.0,
};

fn database_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENFO4_DB") {
        return PathBuf::from(path);
    }
    let root = resolve_asset_root(
        &AssetRootEnv::from_process(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .expect("set OPENFO4_DB or OPENFO4_INSTALL_DIR");
    PathBuf::from(format!("{}/data.db", data_dir(root.to_str().unwrap())))
}

fn centroid(mesh: &NavMesh, tri: u16) -> Vec3 {
    let [a, b, c] = mesh
        .triangle_vertices(tri)
        .expect("link endpoints are in range");
    (a + b + c) / 3.0
}

struct Totals {
    triangles: usize,
    rejected_links: usize,
    duplicate_form_ids: usize,
}

fn load_into_world(meshes: Vec<NavMesh>) -> (NavWorld, Totals) {
    let mut world = NavWorld::new();
    let mut totals = Totals {
        triangles: 0,
        rejected_links: 0,
        duplicate_form_ids: 0,
    };
    for mesh in meshes {
        totals.triangles += mesh.triangle_count();
        match world.add(mesh) {
            Ok(report) => totals.rejected_links += report.rejected.len(),
            Err(WorldError::AlreadyLoaded(_)) => totals.duplicate_form_ids += 1,
            Err(other) => panic!("unexpected world error: {other}"),
        }
    }
    (world, totals)
}

struct ResolvedLink {
    from: NavRef,
    to: NavRef,
    link_type: LinkType,
}

/// Every resolved external link, in a stable order.
fn resolved_links(world: &NavWorld) -> Vec<ResolvedLink> {
    let mut links: Vec<ResolvedLink> = world
        .resident()
        .flat_map(|resident| {
            (0..resident.mesh.triangle_count() as u16).flat_map(move |tri| {
                let from = resident.nav_ref(tri);
                world
                    .links(from)
                    .filter_map(move |link| {
                        link.link_type.map(|link_type| ResolvedLink {
                            from,
                            to: link.target,
                            link_type,
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    links.sort_by_key(|l| (l.from.slot(), l.from.tri(), l.to.slot(), l.to.tri()));
    links
}

fn is_walkable(world: &NavWorld, nav_ref: NavRef) -> bool {
    world
        .mesh(nav_ref)
        .is_some_and(|mesh| mesh.is_walkable(nav_ref.tri()))
}

fn point_on(world: &NavWorld, nav_ref: NavRef) -> NavPoint {
    let mesh = world.mesh(nav_ref).expect("link endpoint is resident");
    NavPoint {
        nav_ref,
        pos: centroid(mesh, nav_ref.tri()),
    }
}

/// Whether a nearest-triangle query under the point finds a surface; the surface may be an
/// overlapping triangle of another mesh, so only its existence is asserted.
fn has_surface_under(world: &NavWorld, point: NavPoint) -> bool {
    find_nearest(world, point.pos, NEAREST, &QueryFilter::new())
        .expect("centroids are finite")
        .is_some_and(|hit| hit.inside)
}

#[test]
#[ignore = "needs FO4 data"]
fn real_navmeshes_decode_link_and_route() {
    let path = database_path();
    let db = DbClient::new(path.to_str().unwrap()).unwrap();
    assert!(
        db.schema().navmesh_columns_present,
        "{} predates navmesh extraction; rerun the installer",
        path.display()
    );
    let records = db.get_all_navmeshes().unwrap();
    let outcome = build_meshes(&records);
    let decode_errors = outcome
        .failures
        .iter()
        .filter(|f| matches!(f.error, BuildError::Decode(_)))
        .count();
    let mesh_errors = outcome.failures.len() - decode_errors;
    for failure in outcome.failures.iter().take(10) {
        println!("failed {}: {}", failure.form_key, failure.error);
    }
    let built = outcome.meshes.len();

    let (world, totals) = load_into_world(outcome.meshes);
    let stats = world.stats();
    let links = resolved_links(&world);
    let ledge_links = links
        .iter()
        .filter(|l| matches!(l.link_type, LinkType::LedgeUp | LinkType::LedgeDown))
        .count();
    let zero_area_links = links
        .iter()
        .filter(|l| !is_walkable(&world, l.from) || !is_walkable(&world, l.to))
        .count();
    println!("records read:           {}", records.len());
    println!("meshes built:           {built}");
    println!("decode errors:          {decode_errors}");
    println!("mesh validation fails:  {mesh_errors}");
    println!("duplicate form ids:     {}", totals.duplicate_form_ids);
    println!("resident meshes:        {}", stats.meshes);
    println!("triangles:              {}", totals.triangles);
    println!("zero-XZ-area triangles: {}", stats.zero_area_triangles);
    println!("one-way neighbour edges: {}", stats.asymmetric_neighbours);
    println!("links resolved:         {}", stats.resolved_links);
    println!("  ledge links:          {ledge_links}");
    println!("  touching zero-area:   {zero_area_links}");
    println!("  portal fallbacks:     {}", stats.portal_fallbacks);
    println!("links pending:          {}", stats.pending_links);
    println!("links rejected:         {}", totals.rejected_links);

    assert!(built > 0, "the database holds no usable navmesh");
    assert_eq!(decode_errors, 0, "decode errors");
    assert_eq!(mesh_errors, 0, "mesh validation failures");
    assert_eq!(totals.duplicate_form_ids, 0, "duplicate form ids");
    assert_eq!(stats.meshes, built);
    assert!(
        !links.is_empty(),
        "no external link resolved between meshes"
    );

    let usable: Vec<&ResolvedLink> = links
        .iter()
        .filter(|l| is_walkable(&world, l.from) && is_walkable(&world, l.to) && l.from != l.to)
        .collect();
    println!(
        "sampled routes skip {} links that touch a zero-area triangle or join a triangle to itself",
        links.len() - usable.len()
    );
    let (ledges, portals): (Vec<&ResolvedLink>, Vec<&ResolvedLink>) = usable
        .into_iter()
        .partition(|l| matches!(l.link_type, LinkType::LedgeUp | LinkType::LedgeDown));
    assert!(!portals.is_empty(), "no portal link to route across");
    let mut search = PathSearch::new(SEARCH_NODE_CAP);
    let mut incomplete_routes = Vec::new();
    let mut sampled = 0;
    for group in [&portals, &ledges] {
        let step = (group.len() / SAMPLED_LINKS).max(1);
        for link in group.iter().step_by(step).take(SAMPLED_LINKS) {
            sampled += 1;
            let (start, goal) = (point_on(&world, link.from), point_on(&world, link.to));
            assert!(
                has_surface_under(&world, start),
                "no surface under {:?}",
                link.from
            );
            assert!(
                has_surface_under(&world, goal),
                "no surface under {:?}",
                link.to
            );
            let result = find_path(&world, &mut search, start, goal, &QueryFilter::new())
                .unwrap_or_else(|e| panic!("search {:?} -> {:?} failed: {e}", link.from, link.to));
            println!(
                "route {:?} -> {:?} ({:?}): {} triangles, {:?}",
                link.from,
                link.to,
                link.link_type,
                result.refs.len(),
                result.completeness
            );
            assert!(
                result.refs.iter().all(|r| world.is_valid(*r)),
                "route holds a stale reference"
            );
            assert_eq!(result.refs.first(), Some(&start.nav_ref));
            if result.completeness == Completeness::Complete {
                assert_eq!(result.refs.last(), Some(&goal.nav_ref));
            } else {
                incomplete_routes.push((link.from, link.to, result.completeness));
            }
        }
    }
    println!("sampled routes: {sampled}");
    println!("incomplete sampled routes: {incomplete_routes:?}");
    assert!(
        incomplete_routes.is_empty(),
        "routes along a resolved link must complete"
    );
}

/// Meshes reachable from `start_mesh` over resolved links, with the first triangle each was entered
/// at and the number of mesh borders crossed to get there.
fn mesh_hops_from(
    adjacency: &HashMap<u32, Vec<(NavRef, NavRef)>>,
    start_mesh: u32,
) -> HashMap<u32, (u32, NavRef)> {
    let mut reached: HashMap<u32, (u32, NavRef)> = HashMap::new();
    let mut queue = VecDeque::from([start_mesh]);
    while let Some(mesh) = queue.pop_front() {
        let hops = reached.get(&mesh).map_or(0, |entry| entry.0);
        for &(_, to) in adjacency.get(&mesh).into_iter().flatten() {
            if to.slot() != start_mesh && !reached.contains_key(&to.slot()) {
                reached.insert(to.slot(), (hops + 1, to));
                queue.push_back(to.slot());
            }
        }
    }
    reached
}

/// Candidate (start, goal) pairs whose meshes lie at least `min_mesh_hops` borders apart.
fn far_apart_pairs(
    world: &NavWorld,
    links: &[ResolvedLink],
    min_mesh_hops: u32,
) -> Vec<(NavPoint, NavPoint)> {
    let mut adjacency: HashMap<u32, Vec<(NavRef, NavRef)>> = HashMap::new();
    for link in links
        .iter()
        .filter(|l| is_walkable(world, l.from) && is_walkable(world, l.to))
    {
        adjacency
            .entry(link.from.slot())
            .or_default()
            .push((link.from, link.to));
    }
    let mut starts: Vec<u32> = adjacency.keys().copied().collect();
    starts.sort_unstable();
    let step = (starts.len() / LONG_ROUTE_CANDIDATES_TRIED).max(1);
    let mut pairs = Vec::new();
    for &start_mesh in starts.iter().step_by(step) {
        let reached = mesh_hops_from(&adjacency, start_mesh);
        let Some(&(from, _)) = adjacency[&start_mesh].first() else {
            continue;
        };
        let farthest = reached
            .values()
            .filter(|(hops, _)| *hops == min_mesh_hops)
            .max_by_key(|(hops, goal)| (*hops, goal.slot(), goal.tri()));
        if let Some(&(_, goal)) = farthest {
            pairs.push((point_on(world, from), point_on(world, goal)));
        }
    }
    pairs
}

/// The funnel output stays a walk except across a ledge, where it jumps from the ledge's crossing
/// point to its landing point.
fn ledge_jumps(world: &NavWorld, corridor: &[NavRef]) -> Vec<(Vec3, Vec3)> {
    corridor
        .windows(2)
        .filter_map(|pair| world.link_between(pair[0], pair[1]))
        .filter_map(|link| link.landing.map(|landing| (link.crossing(), landing)))
        .collect()
}

fn is_ledge_jump(jumps: &[(Vec3, Vec3)], from: Vec3, to: Vec3) -> bool {
    jumps
        .iter()
        .any(|(crossing, landing)| crossing.distance(from) < 1e-3 && landing.distance(to) < 1e-3)
}

/// Whether the XZ projection of the triangle contains `point`, allowing `GRAZE_TOLERANCE` slack so
/// a funnel corner on a shared edge belongs to both triangles.
fn triangle_contains_xz(corners: [Vec3; 3], point: Vec3) -> bool {
    let edge_side =
        |a: Vec3, b: Vec3| (b.x - a.x) * (point.z - a.z) - (b.z - a.z) * (point.x - a.x);
    let sides = [
        (
            edge_side(corners[0], corners[1]),
            corners[0].distance(corners[1]),
        ),
        (
            edge_side(corners[1], corners[2]),
            corners[1].distance(corners[2]),
        ),
        (
            edge_side(corners[2], corners[0]),
            corners[2].distance(corners[0]),
        ),
    ];
    let slack = |(side, length): (f32, f32)| side / length.max(f32::EPSILON);
    sides.iter().all(|&s| slack(s) >= -GRAZE_TOLERANCE)
        || sides.iter().all(|&s| slack(s) <= GRAZE_TOLERANCE)
}

/// Units the straight segment `from -> to` stops short of its end when walked from the best
/// corridor triangle that contains `from`; 0 when it reaches the end unobstructed.
fn straight_segment_shortfall(world: &NavWorld, corridor: &[NavRef], from: Vec3, to: Vec3) -> f32 {
    let length = from.distance(to);
    let starts: Vec<NavRef> = corridor
        .iter()
        .copied()
        .filter(|r| {
            world
                .mesh(*r)
                .and_then(|mesh| mesh.triangle_vertices(r.tri()))
                .is_some_and(|corners| triangle_contains_xz(corners, from))
        })
        .collect();
    assert!(
        !starts.is_empty(),
        "funnel point {from:?} lies in no corridor triangle"
    );
    starts
        .into_iter()
        .map(|nav_ref| {
            let start = NavPoint { nav_ref, pos: from };
            match raycast(world, start, to, &QueryFilter::new(), DEFAULT_MAX_STEPS) {
                Ok(cast) => cast.hit.map_or(0.0, |hit| (1.0 - hit.t) * length),
                Err(RaycastError::DegenerateStart) => f32::INFINITY,
                Err(e) => panic!("raycast {from:?} -> {to:?} failed: {e}"),
            }
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
#[ignore = "needs FO4 data"]
fn long_cross_mesh_routes_complete_and_their_straight_segments_stay_on_the_surface() {
    let path = database_path();
    let db = DbClient::new(path.to_str().unwrap()).unwrap();
    let (world, _) = load_into_world(build_meshes(&db.get_all_navmeshes().unwrap()).meshes);
    let links = resolved_links(&world);

    let mut search = PathSearch::new(LONG_ROUTE_NODE_CAP);
    let mut completed = 0;
    let mut grazes = 0;
    let mut longest_shortfall = 0.0f32;
    let mut incomplete = 0;
    for (start, goal) in far_apart_pairs(&world, &links, LONG_ROUTE_MIN_MESH_HOPS) {
        let result = find_path(&world, &mut search, start, goal, &QueryFilter::new())
            .unwrap_or_else(|e| {
                panic!(
                    "search {:?} -> {:?} failed: {e}",
                    start.nav_ref, goal.nav_ref
                )
            });
        if result.completeness != Completeness::Complete {
            println!(
                "incomplete: {:?} {} refs",
                result.completeness,
                result.refs.len()
            );
            incomplete += 1;
            continue;
        }
        assert!(
            result.refs.len() >= LONG_ROUTE_MIN_TRIANGLES,
            "route {:?} -> {:?} has only {} triangles",
            start.nav_ref,
            goal.nav_ref,
            result.refs.len()
        );
        let points = string_pull(&world, &result.refs, start.pos, goal.pos, AGENT_RADIUS)
            .unwrap_or_else(|e| {
                panic!(
                    "funnel {:?} -> {:?} failed: {e}",
                    start.nav_ref, goal.nav_ref
                )
            });
        assert!(
            points.iter().all(|p| p.is_finite()),
            "funnel produced a non-finite point"
        );
        let jumps = ledge_jumps(&world, &result.refs);
        let (mut segments, mut route_shortfall) = (0, 0.0f32);
        for pair in points.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let length = from.distance(to);
            if length < 1e-3 || is_ledge_jump(&jumps, from, to) {
                continue;
            }
            segments += 1;
            let shortfall = straight_segment_shortfall(&world, &result.refs, from, to);
            longest_shortfall = longest_shortfall.max(shortfall);
            route_shortfall = route_shortfall.max(shortfall);
            assert!(
                shortfall <= GRAZE_TOLERANCE,
                "segment {from:?} -> {to:?} hits a wall {shortfall} units before its end"
            );
            if shortfall > 0.0 {
                grazes += 1;
            }
        }
        println!(
            "long route {:?} -> {:?}: {} triangles, {} funnel points, {segments} straight segments checked, worst shortfall {route_shortfall}",
            start.nav_ref,
            goal.nav_ref,
            result.refs.len(),
            points.len()
        );
        completed += 1;
    }
    println!(
        "long routes completed: {completed}, incomplete candidates: {incomplete}, grazing segments: {grazes}, longest shortfall: {longest_shortfall}"
    );
    assert!(
        completed >= LONG_ROUTES_WANTED,
        "only {completed} long routes completed ({incomplete} candidates incomplete)"
    );
}
