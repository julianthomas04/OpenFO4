//! Debug overlay drawing the resident navigation meshes as a wireframe, coloured by triangle flags.

use std::collections::{HashMap, HashSet};

use glam::Vec3;

use super::debug_view::swatch;
use super::line_buffer::LineBuffer;
use super::render_pipelines::debug_lines::LineVertex;
use crate::navmesh::flags::{
    DELETED, DOOR, EDGE_0_LINK, EDGE_1_LINK, EDGE_2_LINK, NO_LARGE_CREATURES, PREFERRED, WATER,
};
use crate::navmesh::{NavMesh, NavStats, NavWorld, ResidentMesh};

pub const NORMAL: [u8; 4] = [230, 230, 230, 255];
pub const WATER_COLOR: [u8; 4] = [51, 128, 255, 255];
pub const PREFERRED_COLOR: [u8; 4] = [255, 204, 26, 255];
pub const DOOR_COLOR: [u8; 4] = [255, 115, 26, 255];
pub const NO_LARGE_CREATURES_COLOR: [u8; 4] = [255, 64, 64, 255];
pub const LINK_RESOLVED: [u8; 4] = [26, 255, 77, 255];
pub const LINK_PENDING: [u8; 4] = [255, 51, 230, 255];

/// Engine units the lines float above the surface, against z-fighting with the floor geometry.
const LIFT: f32 = 0.08;
/// Height of the marker post standing on each external link edge, in engine units.
const LINK_POST_HEIGHT: f32 = 2.0;
const EDGE_LINK_FLAGS: [u16; 3] = [EDGE_0_LINK, EDGE_1_LINK, EDGE_2_LINK];

/// Flagged triangle colours with their legend labels, in priority order: the first match wins.
const FLAG_LEGEND: [(u16, &str, [u8; 4]); 4] = [
    (WATER, "Water", WATER_COLOR),
    (PREFERRED, "Preferred path", PREFERRED_COLOR),
    (DOOR, "Door", DOOR_COLOR),
    (
        NO_LARGE_CREATURES,
        "No large creatures",
        NO_LARGE_CREATURES_COLOR,
    ),
];

/// Colour of an edge by its triangle's flags; the first match in `FLAG_LEGEND` wins.
pub fn flag_color(flags: u16) -> [u8; 4] {
    FLAG_LEGEND
        .into_iter()
        .find(|(bit, _, _)| flags & bit != 0)
        .map_or(NORMAL, |(_, _, color)| color)
}

/// Every colour the overlay draws, with its legend label.
fn legend_entries() -> impl Iterator<Item = (&'static str, [u8; 4])> {
    [("Walkable", NORMAL)]
        .into_iter()
        .chain(
            FLAG_LEGEND
                .into_iter()
                .map(|(_, label, color)| (label, color)),
        )
        .chain([
            ("Link to loaded navmesh", LINK_RESOLVED),
            ("Link to unloaded navmesh", LINK_PENDING),
        ])
}

/// The Navmesh panel: overlay toggle, residency counts and the colour legend.
pub fn navmesh_section(ui: &mut egui::Ui, overlay: &mut NavmeshOverlay, stats: NavStats) {
    ui.checkbox(&mut overlay.enabled, "Navmesh overlay (N)");
    ui.label(format!(
        "Resident: {} meshes, {} triangles",
        stats.meshes, stats.triangles
    ));
    ui.label(format!(
        "Links: {} resolved, {} pending",
        stats.resolved_links, stats.pending_links
    ));
    ui.label("Triangle colours (first match wins, links drawn as posts):");
    ui.horizontal_wrapped(|ui| {
        for (label, [r, g, b, _]) in legend_entries() {
            swatch(ui, egui::Color32::from_rgb(r, g, b), label);
        }
    });
}

/// An external link edge: its geometry is cached with the mesh, its colour is decided at every
/// build because it follows whether the target mesh is resident.
struct LinkEdge {
    tri: u16,
    edge: u8,
    a: Vec3,
    b: Vec3,
}

/// Everything about one resident mesh that does not depend on its neighbours.
struct MeshLines {
    edges: Vec<LineVertex>,
    links: Vec<LinkEdge>,
}

impl MeshLines {
    fn build(mesh: &NavMesh) -> Self {
        let link_edges: HashSet<(u16, u8)> = mesh
            .edge_links()
            .iter()
            .map(|link| (link.triangle, link.edge))
            .collect();
        let lift = Vec3::Y * LIFT;
        let mut edges = Vec::with_capacity(mesh.triangle_count() * 4);
        let mut links = Vec::with_capacity(link_edges.len());
        for triangle in mesh.triangles() {
            if triangle.flags & DELETED != 0 {
                continue;
            }
            for edge in 0..3u8 {
                let shared_and_drawn_by_neighbour =
                    mesh.neighbour(triangle.index, edge).is_some_and(|n| {
                        n < triangle.index && mesh.flags(n).is_some_and(|f| f & DELETED == 0)
                    });
                if shared_and_drawn_by_neighbour {
                    continue;
                }
                let a = triangle.corners[usize::from(edge)] + lift;
                let b = triangle.corners[(usize::from(edge) + 1) % 3] + lift;
                let flagged = triangle.flags & EDGE_LINK_FLAGS[usize::from(edge)] != 0;
                if flagged && link_edges.contains(&(triangle.index, edge)) {
                    links.push(LinkEdge {
                        tri: triangle.index,
                        edge,
                        a,
                        b,
                    });
                } else {
                    let color = flag_color(triangle.flags);
                    edges.extend([LineVertex::new(a, color), LineVertex::new(b, color)]);
                }
            }
        }
        Self { edges, links }
    }

    fn push_into(&self, world: &NavWorld, resident: &ResidentMesh, lines: &mut Vec<LineVertex>) {
        lines.extend_from_slice(&self.edges);
        for link in &self.links {
            let resolved = world
                .links(resident.nav_ref(link.tri))
                .any(|nav| nav.link_type.is_some() && nav.edge == link.edge);
            let color = if resolved {
                LINK_RESOLVED
            } else {
                LINK_PENDING
            };
            let mid = (link.a + link.b) * 0.5;
            lines.extend([
                LineVertex::new(link.a, color),
                LineVertex::new(link.b, color),
                LineVertex::new(mid, color),
                LineVertex::new(mid + Vec3::Y * LINK_POST_HEIGHT, color),
            ]);
        }
    }
}

/// Per-mesh line geometry keyed by residency, so a mesh is turned into lines once for as long as
/// it stays resident and a residency change only builds the meshes that arrived.
#[derive(Default)]
pub struct LineCache {
    meshes: HashMap<(u32, u16), MeshLines>,
    builds: usize,
}

impl LineCache {
    /// Number of meshes turned into lines so far (cache misses).
    pub fn mesh_builds(&self) -> usize {
        self.builds
    }

    /// Line list of every resident navmesh. Deleted triangles are skipped; an edge shared by two
    /// triangles is emitted once.
    pub fn build(&mut self, world: &NavWorld) -> Vec<LineVertex> {
        let mut previous = std::mem::take(&mut self.meshes);
        let mut lines = Vec::new();
        for resident in world.resident() {
            let key = resident.residency_key();
            let cached = previous.remove(&key).unwrap_or_else(|| {
                self.builds += 1;
                MeshLines::build(resident.mesh)
            });
            cached.push_into(world, &resident, &mut lines);
            self.meshes.insert(key, cached);
        }
        lines
    }

    pub fn clear(&mut self) {
        self.meshes.clear();
    }
}

/// Line list of every resident navmesh, without keeping a cache.
pub fn build_lines(world: &NavWorld) -> Vec<LineVertex> {
    LineCache::default().build(world)
}

/// The overlay's toggle, its GPU line buffer, the per-mesh line cache and the world generation the
/// buffer was built from.
pub struct NavmeshOverlay {
    pub enabled: bool,
    lines: LineBuffer,
    cache: LineCache,
    built_generation: Option<u64>,
}

impl NavmeshOverlay {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            lines: LineBuffer::new("navmesh_overlay_lines"),
            cache: LineCache::default(),
            built_generation: None,
        }
    }

    /// Rebuilds the line buffer in each frame in which the resident set changed since the last
    /// build (only meshes that became resident are converted to lines; the rest is concatenated
    /// from the cache) and frees the buffer and the cache while the overlay is off.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &NavWorld) {
        if !self.enabled {
            self.lines.release();
            self.cache.clear();
            self.built_generation = None;
            return;
        }
        if self.built_generation == Some(world.generation()) {
            return;
        }
        self.lines.upload(device, queue, &self.cache.build(world));
        self.built_generation = Some(world.generation());
    }

    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        pipeline: &'pass wgpu::RenderPipeline,
    ) {
        if self.enabled {
            self.lines.draw(pass, pipeline);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navmesh::flags::EDGE_1_LINK;
    use crate::navmesh::{EdgeLinkSource, NavMesh, NavMeshSource};

    /// Two triangles of a 10 x 10 quad on the XZ plane at height 5, sharing the diagonal.
    fn quad_source(form_id: u32, flags: [u16; 2], links: Vec<EdgeLinkSource>) -> NavMeshSource {
        NavMeshSource {
            form_id,
            vertices: vec![
                Vec3::new(0.0, 5.0, 0.0),
                Vec3::new(10.0, 5.0, 0.0),
                Vec3::new(10.0, 5.0, 10.0),
                Vec3::new(0.0, 5.0, 10.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            neighbours: vec![[-1, -1, 1], [0, -1, -1]],
            flags: flags.to_vec(),
            edge_links: links,
            door_links: Vec::new(),
        }
    }

    fn world_with(sources: Vec<NavMeshSource>) -> NavWorld {
        let mut world = NavWorld::new();
        for source in sources {
            world.add(NavMesh::new(source).unwrap()).unwrap();
        }
        world
    }

    fn colors(lines: &[LineVertex]) -> Vec<[u8; 4]> {
        lines.iter().map(|v| v.color).collect()
    }

    fn count_of(lines: &[LineVertex], color: [u8; 4]) -> usize {
        colors(lines).iter().filter(|c| **c == color).count()
    }

    #[test]
    fn flag_colors_follow_the_documented_priority() {
        assert_eq!(flag_color(0), NORMAL);
        assert_eq!(flag_color(NO_LARGE_CREATURES), NO_LARGE_CREATURES_COLOR);
        assert_eq!(flag_color(NO_LARGE_CREATURES | DOOR), DOOR_COLOR);
        assert_eq!(flag_color(DOOR | PREFERRED), PREFERRED_COLOR);
        assert_eq!(flag_color(PREFERRED | WATER), WATER_COLOR);
    }

    #[test]
    fn shared_edges_are_emitted_once() {
        let world = world_with(vec![quad_source(1, [0, 0], Vec::new())]);
        // Four boundary edges plus the shared diagonal, two vertices each.
        assert_eq!(build_lines(&world).len(), 10);
    }

    #[test]
    fn lines_float_above_the_surface() {
        let world = world_with(vec![quad_source(1, [0, 0], Vec::new())]);
        assert!(
            build_lines(&world)
                .iter()
                .all(|v| (v.position[1] - (5.0 + LIFT)).abs() < 1e-6)
        );
    }

    #[test]
    fn edges_take_the_colour_of_their_triangle() {
        let world = world_with(vec![quad_source(1, [WATER, 0], Vec::new())]);
        let lines = build_lines(&world);
        // Triangle 0 owns its two boundary edges and the diagonal.
        assert_eq!(count_of(&lines, WATER_COLOR), 6);
        assert_eq!(count_of(&lines, NORMAL), 4);
    }

    #[test]
    fn deleted_triangles_are_not_drawn_and_hand_back_shared_edges() {
        let world = world_with(vec![quad_source(1, [0, DELETED], Vec::new())]);
        // Triangle 0 now draws the diagonal itself: three edges, no triangle 1.
        assert_eq!(build_lines(&world).len(), 6);
    }

    fn linked_source(form_id: u32, target_form_id: u32) -> NavMeshSource {
        let link = EdgeLinkSource {
            triangle: 1,
            edge: 1,
            link_type: 0,
            target_form_id,
            target_triangle: 0,
            target_edge: 0,
        };
        quad_source(form_id, [0, EDGE_1_LINK], vec![link])
    }

    #[test]
    fn external_links_are_coloured_by_resolution_and_carry_a_post() {
        let pending = world_with(vec![linked_source(1, 2)]);
        let lines = build_lines(&pending);
        assert_eq!(count_of(&lines, LINK_PENDING), 4);
        assert_eq!(count_of(&lines, LINK_RESOLVED), 0);

        let resolved = world_with(vec![
            linked_source(1, 2),
            quad_source(2, [0, 0], Vec::new()),
        ]);
        let lines = build_lines(&resolved);
        assert_eq!(count_of(&lines, LINK_RESOLVED), 4);
        assert_eq!(count_of(&lines, LINK_PENDING), 0);
        let post_top = lines.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
        assert!((post_top - (5.0 + LIFT + LINK_POST_HEIGHT)).abs() < 1e-5);
    }

    #[test]
    fn unloading_the_target_turns_a_resolved_link_pending() {
        let mut world = world_with(vec![
            linked_source(1, 2),
            quad_source(2, [0, 0], Vec::new()),
        ]);
        world.remove(2).unwrap();
        assert!(count_of(&build_lines(&world), LINK_PENDING) > 0);
    }

    #[test]
    fn an_unchanged_world_is_converted_to_lines_once() {
        let world = world_with(vec![
            quad_source(1, [0, 0], Vec::new()),
            quad_source(2, [0, 0], Vec::new()),
        ]);
        let mut cache = LineCache::default();
        let first = cache.build(&world);
        assert_eq!(cache.mesh_builds(), 2);
        let second = cache.build(&world);
        assert_eq!(
            cache.mesh_builds(),
            2,
            "the second build is served from the cache"
        );
        assert_eq!(colors(&first), colors(&second));
        assert_eq!(first.len(), second.len());
    }

    #[test]
    fn a_new_mesh_is_the_only_one_converted_and_the_cache_matches_a_fresh_build() {
        let mut world = world_with(vec![quad_source(1, [WATER, 0], Vec::new())]);
        let mut cache = LineCache::default();
        cache.build(&world);
        world
            .add(NavMesh::new(quad_source(2, [0, PREFERRED], Vec::new())).unwrap())
            .unwrap();
        let cached = cache.build(&world);
        assert_eq!(cache.mesh_builds(), 2);
        let fresh = build_lines(&world);
        assert_eq!(colors(&cached), colors(&fresh));
        assert_eq!(cached.len(), fresh.len());
    }

    #[test]
    fn a_removed_mesh_leaves_the_lines_and_a_reload_is_converted_again() {
        let mut world = world_with(vec![
            quad_source(1, [0, 0], Vec::new()),
            quad_source(2, [0, 0], Vec::new()),
        ]);
        let mut cache = LineCache::default();
        assert_eq!(cache.build(&world).len(), 20);
        world.remove(2).unwrap();
        assert_eq!(cache.build(&world).len(), 10);
        assert_eq!(cache.mesh_builds(), 2);
        world
            .add(NavMesh::new(quad_source(3, [WATER, 0], Vec::new())).unwrap())
            .unwrap();
        let lines = cache.build(&world);
        assert_eq!(lines.len(), 20);
        assert_eq!(
            cache.mesh_builds(),
            3,
            "the new mesh reuses the slot but is a new residency"
        );
        assert_eq!(count_of(&lines, WATER_COLOR), 6);
    }

    #[test]
    fn link_colours_follow_the_target_without_rebuilding_the_source() {
        let mut world = world_with(vec![linked_source(1, 2)]);
        let mut cache = LineCache::default();
        assert_eq!(count_of(&cache.build(&world), LINK_PENDING), 4);
        world
            .add(NavMesh::new(quad_source(2, [0, 0], Vec::new())).unwrap())
            .unwrap();
        let lines = cache.build(&world);
        assert_eq!(count_of(&lines, LINK_RESOLVED), 4);
        assert_eq!(count_of(&lines, LINK_PENDING), 0);
        assert_eq!(
            cache.mesh_builds(),
            2,
            "only the arriving mesh was converted"
        );
        world.remove(2).unwrap();
        assert_eq!(count_of(&cache.build(&world), LINK_PENDING), 4);
        assert_eq!(cache.mesh_builds(), 2);
    }

    #[test]
    fn only_flagged_edges_are_drawn_as_links() {
        let link = EdgeLinkSource {
            triangle: 1,
            edge: 1,
            link_type: 0,
            target_form_id: 2,
            target_triangle: 0,
            target_edge: 0,
        };
        let unflagged = world_with(vec![quad_source(1, [0, 0], vec![link])]);
        let lines = build_lines(&unflagged);
        assert_eq!(
            count_of(&lines, LINK_PENDING) + count_of(&lines, LINK_RESOLVED),
            0
        );
        let flagged_without_link = world_with(vec![quad_source(1, [0, EDGE_1_LINK], Vec::new())]);
        let lines = build_lines(&flagged_without_link);
        assert_eq!(
            count_of(&lines, LINK_PENDING) + count_of(&lines, LINK_RESOLVED),
            0
        );
        assert_eq!(lines.len(), 10);
    }

    #[test]
    fn empty_world_has_no_lines() {
        assert!(build_lines(&NavWorld::new()).is_empty());
    }
}
