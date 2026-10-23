use std::collections::HashMap;

use glam::Vec3;

use super::link::{EdgeLink, LinkType};
use super::mesh::NavMesh;
use super::nav_ref::NavRef;
use super::portal::{Span, clip_portal, full_edge};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorldError {
    #[error("navmesh {0:#010x} is already loaded")]
    AlreadyLoaded(u32),
    #[error("navmesh {0:#010x} is not loaded")]
    NotLoaded(u32),
}

/// An authored link that could not be activated against its resident target because the target
/// has no such triangle or edge. It stays pending, so reloading the target (for example as a
/// different, larger mesh) retries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectedLink {
    pub source_form_id: u32,
    pub authored_index: usize,
}

/// Outcome of loading a mesh: how many authored links (its own or other meshes' waiting for it)
/// became active, were deferred until their target loads, or were rejected against a resident
/// target (rejected links stay pending).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadReport {
    pub slot: u32,
    pub salt: u16,
    pub linked: usize,
    pub deferred: usize,
    pub rejected: Vec<RejectedLink>,
    /// Newly active portal links whose edges did not overlap, so the shorter edge was used.
    pub portal_fallbacks: usize,
    /// Edges of the loaded mesh whose neighbour does not list them back.
    pub asymmetric_neighbours: usize,
    /// Triangles of the loaded mesh without XZ area.
    pub zero_area_triangles: usize,
}

/// Residency totals for logs and the debug UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavStats {
    pub meshes: usize,
    pub triangles: usize,
    /// Triangles without XZ area; queries treat them as impassable.
    pub zero_area_triangles: usize,
    pub asymmetric_neighbours: usize,
    pub resolved_links: usize,
    /// Links waiting for their target mesh, including links rejected against a resident target.
    pub pending_links: usize,
    /// Resolved portal links whose edges did not overlap.
    pub portal_fallbacks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnloadReport {
    pub links_deferred: usize,
}

/// A traversable connection out of a triangle; `link_type` is `None` for internal adjacency.
///
/// `span` is the crossable stretch of the source edge (the whole edge for internal adjacency, the
/// overlap with the target edge for portals). A ledge link has no shared stretch: it is crossed at
/// the middle of its source edge and lands at `landing`, the middle of the target edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavLink {
    pub target: NavRef,
    pub edge: u8,
    pub link_type: Option<LinkType>,
    pub span: Span,
    pub landing: Option<Vec3>,
}

impl NavLink {
    pub fn crossing(&self) -> Vec3 {
        self.span.midpoint()
    }

    pub fn is_ledge(&self) -> bool {
        self.landing.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResidentMesh<'a> {
    pub mesh: &'a NavMesh,
    slot: u32,
    salt: u16,
}

impl ResidentMesh<'_> {
    /// Identity of this residency: a reload of the same form id gets a different one.
    pub fn residency_key(&self) -> (u32, u16) {
        (self.slot, self.salt)
    }

    pub fn nav_ref(&self, tri: u16) -> NavRef {
        NavRef::new(self.slot, self.salt, tri)
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedLink {
    tri: u16,
    edge: u8,
    link_type: LinkType,
    target: NavRef,
    authored_index: u32,
    span: Span,
    landing: Option<Vec3>,
    fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinkKey {
    source_slot: u32,
    authored_index: u32,
}

#[derive(Debug)]
struct LoadedMesh {
    mesh: NavMesh,
    external: Vec<ResolvedLink>,
    back_links: Vec<LinkKey>,
}

impl LoadedMesh {
    fn external_for(&self, tri: u16) -> &[ResolvedLink] {
        let lo = self.external.partition_point(|l| l.tri < tri);
        let hi = self.external.partition_point(|l| l.tri <= tri);
        &self.external[lo..hi]
    }
}

#[derive(Debug)]
struct Slot {
    salt: u16,
    loaded: Option<LoadedMesh>,
}

enum LinkOutcome {
    Resolved { fallback: bool },
    Deferred,
    Rejected,
}

/// All resident navmeshes plus the authored cross-mesh links between them.
///
/// Links are resolved as meshes load and unload; links whose target is not resident (or lacks the
/// linked triangle) wait in a pending list. A reference whose mesh was unloaded fails `is_valid`
/// instead of dangling.
#[derive(Debug, Default)]
pub struct NavWorld {
    slots: Vec<Slot>,
    free_slots: Vec<u32>,
    by_form_id: HashMap<u32, u32>,
    pending: Vec<LinkKey>,
    generation: u64,
}

fn next_salt(salt: u16) -> u16 {
    match salt.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

impl NavWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resident_count(&self) -> usize {
        self.by_form_id.len()
    }

    pub fn pending_link_count(&self) -> usize {
        self.pending.len()
    }

    /// Authored links currently traversable, counted once per direction they were authored in.
    pub fn resolved_link_count(&self) -> usize {
        self.loaded_meshes()
            .map(|loaded| loaded.external.len())
            .sum()
    }

    pub fn stats(&self) -> NavStats {
        NavStats {
            meshes: self.resident_count(),
            triangles: self.resident().map(|r| r.mesh.triangle_count()).sum(),
            zero_area_triangles: self
                .resident()
                .map(|r| r.mesh.zero_area_triangle_count())
                .sum(),
            asymmetric_neighbours: self
                .resident()
                .map(|r| r.mesh.asymmetric_neighbour_count())
                .sum(),
            resolved_links: self.resolved_link_count(),
            pending_links: self.pending_link_count(),
            portal_fallbacks: self
                .loaded_meshes()
                .flat_map(|loaded| &loaded.external)
                .filter(|link| link.fallback)
                .count(),
        }
    }

    fn loaded_meshes(&self) -> impl Iterator<Item = &LoadedMesh> {
        self.slots.iter().filter_map(|entry| entry.loaded.as_ref())
    }

    /// Changes whenever a mesh is added or removed, so a consumer can cache work derived from the
    /// resident set and rebuild only when this value differs.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Drops every resident mesh and pending link. Outstanding references turn stale.
    pub fn clear(&mut self) {
        for entry in &mut self.slots {
            if entry.loaded.take().is_some() {
                entry.salt = next_salt(entry.salt);
            }
        }
        self.free_slots = (0..self.slots.len() as u32).collect();
        self.by_form_id.clear();
        self.pending.clear();
        self.generation += 1;
    }

    pub fn add(&mut self, mesh: NavMesh) -> Result<LoadReport, WorldError> {
        let form_id = mesh.form_id();
        if self.by_form_id.contains_key(&form_id) {
            return Err(WorldError::AlreadyLoaded(form_id));
        }
        let own_links = mesh.edge_links().len();
        let mut report = LoadReport {
            asymmetric_neighbours: mesh.asymmetric_neighbour_count(),
            zero_area_triangles: mesh.zero_area_triangle_count(),
            ..LoadReport::default()
        };
        let slot = self.allocate_slot();
        self.slots[slot as usize].loaded = Some(LoadedMesh {
            mesh,
            external: Vec::new(),
            back_links: Vec::new(),
        });
        self.by_form_id.insert(form_id, slot);
        self.generation += 1;
        report.slot = slot;
        report.salt = self.slots[slot as usize].salt;
        let waiting = self.take_pending_for(form_id);
        let own = (0..own_links).map(|index| LinkKey {
            source_slot: slot,
            authored_index: index as u32,
        });
        for key in own.chain(waiting) {
            match self.resolve_link(key) {
                LinkOutcome::Resolved { fallback } => {
                    report.linked += 1;
                    report.portal_fallbacks += usize::from(fallback);
                }
                LinkOutcome::Deferred => report.deferred += 1,
                LinkOutcome::Rejected => report.rejected.push(self.rejected_link(key)),
            }
        }
        Ok(report)
    }

    pub fn remove(&mut self, form_id: u32) -> Result<UnloadReport, WorldError> {
        let slot = self
            .by_form_id
            .remove(&form_id)
            .ok_or(WorldError::NotLoaded(form_id))?;
        let loaded = self.slots[slot as usize]
            .loaded
            .take()
            .expect("the form id map only names slots holding a mesh");
        let mut report = UnloadReport::default();
        for back in loaded.back_links.iter().filter(|b| b.source_slot != slot) {
            self.unresolve_link(*back);
            self.pending.push(*back);
            report.links_deferred += 1;
        }
        for link in loaded.external.iter().filter(|l| l.target.slot() != slot) {
            self.drop_back_link(link.target.slot(), slot, link.authored_index);
        }
        self.pending.retain(|p| p.source_slot != slot);
        self.generation += 1;
        let entry = &mut self.slots[slot as usize];
        entry.salt = next_salt(entry.salt);
        self.free_slots.push(slot);
        Ok(report)
    }

    pub fn is_valid(&self, nav_ref: NavRef) -> bool {
        self.mesh(nav_ref)
            .is_some_and(|mesh| mesh.contains_triangle(nav_ref.tri()))
    }

    pub fn mesh(&self, nav_ref: NavRef) -> Option<&NavMesh> {
        self.loaded(nav_ref).map(|loaded| &loaded.mesh)
    }

    pub fn nav_ref(&self, form_id: u32, tri: u16) -> Option<NavRef> {
        let slot = *self.by_form_id.get(&form_id)?;
        let entry = &self.slots[slot as usize];
        let loaded = entry.loaded.as_ref()?;
        loaded
            .mesh
            .contains_triangle(tri)
            .then(|| NavRef::new(slot, entry.salt, tri))
    }

    pub fn resident(&self) -> impl Iterator<Item = ResidentMesh<'_>> {
        self.slots.iter().enumerate().filter_map(|(slot, entry)| {
            entry.loaded.as_ref().map(|loaded| ResidentMesh {
                mesh: &loaded.mesh,
                slot: slot as u32,
                salt: entry.salt,
            })
        })
    }

    /// Connections out of `nav_ref`: internal neighbours followed by resolved external links.
    /// Empty for a stale reference.
    pub fn links(&self, nav_ref: NavRef) -> impl Iterator<Item = NavLink> + '_ {
        let tri = nav_ref.tri();
        self.loaded(nav_ref)
            .filter(move |loaded| loaded.mesh.contains_triangle(tri))
            .into_iter()
            .flat_map(move |loaded| {
                let (slot, salt) = (nav_ref.slot(), nav_ref.salt());
                let internal = (0..3u8).filter_map(move |edge| {
                    let neighbour = loaded.mesh.neighbour(tri, edge)?;
                    let edge_corners = loaded.mesh.edge_vertices(tri, edge)?;
                    Some(NavLink {
                        target: NavRef::new(slot, salt, neighbour),
                        edge,
                        link_type: None,
                        span: full_edge(edge_corners),
                        landing: None,
                    })
                });
                let external = loaded.external_for(tri).iter().map(|l| NavLink {
                    target: l.target,
                    edge: l.edge,
                    link_type: Some(l.link_type),
                    span: l.span,
                    landing: l.landing,
                });
                internal.chain(external)
            })
    }

    pub fn link_between(&self, from: NavRef, to: NavRef) -> Option<NavLink> {
        self.links(from).find(|link| link.target == to)
    }

    fn loaded(&self, nav_ref: NavRef) -> Option<&LoadedMesh> {
        let entry = self.slots.get(nav_ref.slot() as usize)?;
        (entry.salt == nav_ref.salt())
            .then_some(entry.loaded.as_ref())
            .flatten()
    }

    fn allocate_slot(&mut self) -> u32 {
        self.free_slots.pop().unwrap_or_else(|| {
            self.slots.push(Slot {
                salt: 1,
                loaded: None,
            });
            (self.slots.len() - 1) as u32
        })
    }

    fn take_pending_for(&mut self, target_form_id: u32) -> Vec<LinkKey> {
        let (waiting, rest) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|key| self.authored_link(*key).target_form_id == target_form_id);
        self.pending = rest;
        waiting
    }

    fn loaded_at(&self, slot: u32) -> &LoadedMesh {
        self.slots[slot as usize]
            .loaded
            .as_ref()
            .expect("link keys only name resident meshes: unloading purges them")
    }

    fn loaded_at_mut(&mut self, slot: u32) -> &mut LoadedMesh {
        self.slots[slot as usize]
            .loaded
            .as_mut()
            .expect("link keys only name resident meshes: unloading purges them")
    }

    fn authored_link(&self, key: LinkKey) -> EdgeLink {
        self.loaded_at(key.source_slot).mesh.edge_links()[key.authored_index as usize]
    }

    fn rejected_link(&self, key: LinkKey) -> RejectedLink {
        RejectedLink {
            source_form_id: self.loaded_at(key.source_slot).mesh.form_id(),
            authored_index: key.authored_index as usize,
        }
    }

    /// The single place where an authored link is turned into a traversable connection.
    ///
    /// A link whose target is not resident, or is resident without the linked triangle, stays in
    /// the pending list: the target may unload and come back as a different mesh.
    fn resolve_link(&mut self, key: LinkKey) -> LinkOutcome {
        let link = self.authored_link(key);
        let Some(&target_slot) = self.by_form_id.get(&link.target_form_id) else {
            self.pending.push(key);
            return LinkOutcome::Deferred;
        };
        let target = self.loaded_at(target_slot);
        let source = self.loaded_at(key.source_slot);
        let Some((span, landing, fallback)) = link_geometry(&source.mesh, &link, &target.mesh)
        else {
            self.pending.push(key);
            return LinkOutcome::Rejected;
        };
        let resolved = ResolvedLink {
            tri: link.triangle,
            edge: link.edge,
            link_type: link.link_type,
            target: NavRef::new(
                target_slot,
                self.slots[target_slot as usize].salt,
                link.target_triangle,
            ),
            authored_index: key.authored_index,
            span,
            landing,
            fallback,
        };
        let source = self.loaded_at_mut(key.source_slot);
        let at = source.external.partition_point(|l| l.tri <= resolved.tri);
        source.external.insert(at, resolved);
        self.loaded_at_mut(target_slot).back_links.push(key);
        LinkOutcome::Resolved { fallback }
    }

    fn unresolve_link(&mut self, key: LinkKey) {
        self.loaded_at_mut(key.source_slot)
            .external
            .retain(|l| l.authored_index != key.authored_index);
    }

    fn drop_back_link(&mut self, target_slot: u32, source_slot: u32, authored_index: u32) {
        self.loaded_at_mut(target_slot)
            .back_links
            .retain(|b| !(b.source_slot == source_slot && b.authored_index == authored_index));
    }
}

/// Crossing geometry of an authored link, or `None` when the target mesh lacks the linked
/// triangle or edge. Portals cross where source and target edge overlap; ledges cross at the
/// middle of the source edge and land at the middle of the target edge.
fn link_geometry(
    source: &NavMesh,
    link: &EdgeLink,
    target: &NavMesh,
) -> Option<(Span, Option<Vec3>, bool)> {
    let source_edge = source.edge_vertices(link.triangle, link.edge)?;
    let target_edge = target.edge_vertices(link.target_triangle, link.target_edge)?;
    Some(match link.link_type {
        LinkType::Portal | LinkType::EnableDisablePortal => {
            let clipped = clip_portal(source_edge, target_edge);
            (clipped.span, None, clipped.fallback)
        }
        LinkType::LedgeUp | LinkType::LedgeDown => (
            full_edge(source_edge),
            Some(target_edge.0.midpoint(target_edge.1)),
            false,
        ),
    })
}
