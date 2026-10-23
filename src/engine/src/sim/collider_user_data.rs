//! Bit layout of the `user_data` every collider carries, shared by the
//! physics context that writes it and the ragdoll diagnostics that read it.

use std::collections::HashMap;

use super::CellGrid;

/// Collider `user_data` layout: bits 0..64 = grid coords, bits 64..96 =
/// registry slot of the (mesh path, material) pair (slots start at 1; 0 =
/// terrain / unattributed), bits 96..120 = 24-bit REFR form id, bits
/// 120..128 = plugin registry index + 1 (0 = no ref attribution).
pub(super) fn pack_grid(grid: CellGrid) -> u128 {
    (u128::from(grid.0 as u32) << 32) | u128::from(grid.1 as u32)
}

/// The grid coordinates `pack_grid` stored.
pub(super) fn unpack_grid(user_data: u128) -> CellGrid {
    ((user_data >> 32) as u32 as i32, user_data as u32 as i32)
}

/// The `user_data` of every collider of one placement: the placement's grid and ref bits plus the
/// registry slot of the material each collider's shape carries.
#[derive(Clone)]
pub(super) struct ColliderTag {
    base: u128,
    /// Slot of (mesh path, material 0): the colliders of a shape without a material.
    unmaterialed_slot: u32,
    slots: HashMap<u32, u32>,
}

impl ColliderTag {
    /// `slot_of` yields the registry slot of each material in `materials`; material 0 is always
    /// covered.
    pub(super) fn new(
        base: u128,
        materials: impl IntoIterator<Item = u32>,
        mut slot_of: impl FnMut(u32) -> u32,
    ) -> Self {
        let mut slots = HashMap::new();
        for material in std::iter::once(0).chain(materials) {
            slots.entry(material).or_insert_with(|| slot_of(material));
        }
        Self {
            base,
            unmaterialed_slot: slots[&0],
            slots,
        }
    }

    /// A material the tag was not built for is reported once and attributed like a shape without
    /// material.
    pub(super) fn user_data(&self, material: u32) -> u128 {
        let slot = self.slots.get(&material).copied().unwrap_or_else(|| {
            crate::utils::warn_once(&format!("collider-tag:{material}"), || {
                format!(
                    "collision: material {material:08X} has no registry slot in its placement's \
                     collider tag; its colliders report no material"
                )
            });
            self.unmaterialed_slot
        });
        self.base | pack_path(slot)
    }
}

pub(super) fn pack_path(path_slot: u32) -> u128 {
    u128::from(path_slot) << 64
}

pub(super) fn unpack_path(user_data: u128) -> u32 {
    (user_data >> 64) as u32
}

pub(super) fn pack_ref(form_id: u32, plugin_slot: u32) -> u128 {
    (u128::from(form_id & 0xFF_FFFF) << 96) | (u128::from(plugin_slot) << 120)
}

/// `(form_id, plugin_slot)`; `plugin_slot` 0 = no ref attribution.
pub(super) fn unpack_ref(user_data: u128) -> (u32, u32) {
    (
        ((user_data >> 96) as u32) & 0xFF_FFFF,
        (user_data >> 120) as u32,
    )
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    #[test]
    fn grid_round_trips_negative_coordinates() {
        assert_eq!(unpack_grid(pack_grid((-3, 7))), (-3, 7));
    }

    #[test]
    fn path_slot_survives_alongside_the_grid() {
        let data = pack_grid((12, -5)) | pack_path(0xDEAD_BEEF);
        assert_eq!(unpack_path(data), 0xDEAD_BEEF);
    }

    #[test]
    fn ref_attribution_round_trips_form_id_and_plugin() {
        let data = pack_grid((1, 2)) | pack_path(9) | pack_ref(0x001E_D5A7, 3);
        assert_eq!(unpack_ref(data), (0x001E_D5A7, 3));
        assert_eq!(unpack_path(data), 9);
    }

    #[test]
    fn form_id_is_limited_to_24_bits_and_cannot_corrupt_the_plugin_slot() {
        let (form_id, plugin) = unpack_ref(pack_ref(0xFFFF_FFFF, 1));
        assert_eq!(form_id, 0xFF_FFFF);
        assert_eq!(plugin, 1);
    }

    #[test]
    fn a_tag_gives_each_material_its_own_slot_beside_the_base_bits() {
        let base = pack_grid((2, 3)) | pack_ref(0x0A_0001, 1);
        let tag = ColliderTag::new(base, [7, 9], |material| material + 100);
        for (material, slot) in [(0, 100), (7, 107), (9, 109)] {
            let data = tag.user_data(material);
            assert_eq!(unpack_path(data), slot, "material {material}");
            assert_eq!(unpack_ref(data), (0x0A_0001, 1));
            assert_eq!(data & 0xFFFF_FFFF_FFFF_FFFF, pack_grid((2, 3)));
        }
    }

    #[test]
    fn a_tag_asks_for_each_material_once() {
        let mut asked = Vec::new();
        ColliderTag::new(0, [7, 7, 9], |material| {
            asked.push(material);
            1
        });
        asked.sort_unstable();
        assert_eq!(asked, [0, 7, 9]);
    }

    #[test]
    fn a_material_outside_the_tag_is_attributed_like_no_material() {
        let tag = ColliderTag::new(0, [7], |material| material + 100);
        assert_eq!(unpack_path(tag.user_data(0xFEED_0001)), 100);
    }

    #[test]
    fn terrain_has_no_attribution() {
        let data = pack_grid((4, 4));
        assert_eq!(unpack_path(data), 0);
        assert_eq!(unpack_ref(data), (0, 0));
    }
}
