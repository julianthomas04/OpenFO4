use std::collections::HashSet;
use std::sync::Arc;

use super::flags;
use super::link::LinkType;
use super::mesh::NavMesh;

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum FilterError {
    #[error("flag {0:#06x} must be exactly one bit")]
    NotSingleFlag(u16),
    #[error("cost multiplier {0} must be finite and positive")]
    InvalidMultiplier(f32),
}

/// Which triangles and links a query may use, and what they cost.
///
/// Cost is a per-flag multiplier on travelled distance; a triangle's multiplier is the product of
/// the multipliers of its set flags. Deleted triangles and triangles without area on the XZ plane
/// are always excluded. A triangle with several doors is excluded as soon as the filter blocks any
/// one of them.
///
/// Cloning is cheap (the blocked door set is shared), so one filter can serve many queries.
#[derive(Debug, Clone)]
pub struct QueryFilter {
    multipliers: [f32; 16],
    non_default: u16,
    excluded_flags: u16,
    excluded_link_types: u8,
    blocked_doors: Arc<HashSet<u32>>,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self {
            multipliers: [1.0; 16],
            non_default: 0,
            excluded_flags: 0,
            excluded_link_types: 0,
            blocked_doors: Arc::new(HashSet::new()),
        }
    }
}

impl QueryFilter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn for_large_creature() -> Self {
        Self::default().excluding_flags(flags::NO_LARGE_CREATURES)
    }

    #[must_use]
    pub fn excluding_flags(mut self, flag_bits: u16) -> Self {
        self.excluded_flags |= flag_bits;
        self
    }

    #[must_use]
    pub fn excluding_link_type(mut self, link_type: LinkType) -> Self {
        self.excluded_link_types |= link_type.mask();
        self
    }

    #[must_use]
    pub fn blocking_door(mut self, door_ref_form_id: u32) -> Self {
        Arc::make_mut(&mut self.blocked_doors).insert(door_ref_form_id);
        self
    }

    pub fn with_multiplier(mut self, flag: u16, multiplier: f32) -> Result<Self, FilterError> {
        if flag.count_ones() != 1 {
            return Err(FilterError::NotSingleFlag(flag));
        }
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return Err(FilterError::InvalidMultiplier(multiplier));
        }
        self.multipliers[flag.trailing_zeros() as usize] = multiplier;
        self.non_default |= flag;
        Ok(self)
    }

    pub fn is_passable(&self, mesh: &NavMesh, tri: u16) -> bool {
        self.traversal_multiplier(mesh, tri).is_some()
    }

    /// The cost multiplier of a triangle, or `None` when the filter does not let queries use it
    /// (out of range, deleted, no XZ area, excluded flag or blocked door).
    pub fn traversal_multiplier(&self, mesh: &NavMesh, tri: u16) -> Option<f32> {
        let tri_flags = mesh.flags(tri)?;
        if tri_flags & (flags::DELETED | self.excluded_flags) != 0 || !mesh.is_walkable(tri) {
            return None;
        }
        let door_blocked = !self.blocked_doors.is_empty()
            && mesh
                .door_refs(tri)
                .iter()
                .any(|door| self.blocked_doors.contains(door));
        (!door_blocked).then(|| self.cost_multiplier(tri_flags))
    }

    pub fn allows_link(&self, link_type: Option<LinkType>) -> bool {
        link_type.is_none_or(|kind| self.excluded_link_types & kind.mask() == 0)
    }

    pub fn cost_multiplier(&self, tri_flags: u16) -> f32 {
        let active = tri_flags & self.non_default;
        if active == 0 {
            return 1.0;
        }
        (0..16)
            .filter(|bit| active & (1 << bit) != 0)
            .map(|bit| self.multipliers[bit])
            .product()
    }

    /// Smallest multiplier any triangle can have, capped at 1; scales an admissible heuristic.
    pub fn min_cost_multiplier(&self) -> f32 {
        let cheapest_product: f32 = self.multipliers.iter().filter(|m| **m < 1.0).product();
        cheapest_product.min(1.0)
    }
}
