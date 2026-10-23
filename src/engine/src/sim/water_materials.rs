//! Per-body inputs of the Fallout 4 buoyancy factor: which collision layers the
//! water pass tracks, the material buoyancy average and the ragdoll part-id
//! multipliers. Pure data and arithmetic, no physics types, so
//! the sidecar readers can share [`WaterBodyProps`].

use std::collections::HashMap;

/// `fBuoyancyMultBody` (game default; the ESM carries no override).
const BUOYANCY_MULT_BODY: f32 = 1.0;
/// `fBuoyancyMultExtremity`.
const BUOYANCY_MULT_EXTREMITY: f32 = -0.3;
/// Multiplier of a body that is not a ragdoll limb, and of a limb part id outside 1..=22.
const BUOYANCY_MULT_NEUTRAL: f32 = 1.0;

/// Havok collision layers whose bodies the engine starts tracking when they
/// overlap a water volume: clutter (4), weapon (5), projectile (6), biped (8),
/// props (10), 12, 13, 14, clutter-large (29), dead biped (32), biped without
/// character controller (33). Every other layer is ignored. Layer names are
/// assumed from the Skyrim/FO4 layer enum.
const TRACKED_LAYERS: [u8; 11] = [4, 5, 6, 8, 10, 12, 13, 14, 29, 32, 33];
/// Layers whose bodies are ragdoll limbs and take the part-id multiplier:
/// biped (8), dead biped (0x20), biped without character controller (0x21).
const RAGDOLL_LAYERS: [u8; 3] = [8, 0x20, 0x21];

/// How a ragdoll part contributes to buoyancy, indexed by `part id - 1`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum PartKind {
    Body,
    Extremity,
    Zero,
}

const PART_KINDS: [PartKind; 22] = {
    use PartKind::{Body, Extremity, Zero};
    [
        Body, Body, Body, Body, Body, Extremity, Extremity, Body, Extremity, Extremity, Body,
        Extremity, Extremity, Body, Extremity, Extremity, Zero, Zero, Zero, Zero, Zero, Extremity,
    ]
};

/// Whether the water pass tracks a body of this collision layer.
pub fn is_tracked_layer(layer: u8) -> bool {
    TRACKED_LAYERS.contains(&layer)
}

/// The multiplier applied to the material buoyancy `material` of a body on
/// `layer` with ragdoll part id `part` (filter bits 8-12).
pub fn buoyancy_multiplier(layer: u8, part: u8, material: f32) -> f32 {
    if !RAGDOLL_LAYERS.contains(&layer) {
        return BUOYANCY_MULT_NEUTRAL;
    }
    if material <= 0.0 {
        return BUOYANCY_MULT_BODY;
    }
    let kind = usize::from(part)
        .checked_sub(1)
        .and_then(|index| PART_KINDS.get(index));
    match kind {
        Some(PartKind::Body) => BUOYANCY_MULT_BODY,
        Some(PartKind::Extremity) => BUOYANCY_MULT_EXTREMITY,
        Some(PartKind::Zero) => 0.0,
        None => BUOYANCY_MULT_NEUTRAL,
    }
}

/// What the collision sidecar says about one Havok body (`water` object of the
/// `.col.json`, or of a manifest row): its layer, ragdoll part id and the
/// histogram of its shapes' material hashes as `(hash, shape count)`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct WaterBodyProps {
    pub layer: u8,
    pub part: u8,
    pub materials: Vec<(u32, u32)>,
}

/// MATT `BNAM` buoyancy by material hash (`material_types.material_hash`).
#[derive(Debug, Default, Clone)]
pub struct MaterialBuoyancy {
    by_hash: HashMap<u32, f32>,
}

impl MaterialBuoyancy {
    pub fn new(entries: impl IntoIterator<Item = (u32, f32)>) -> Self {
        Self {
            by_hash: entries.into_iter().collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// The average buoyancy over a body's shapes. A hash without a MATT record
    /// contributes 0, and so does a body without shapes.
    pub fn mean(&self, materials: &[(u32, u32)]) -> f32 {
        let shapes: u32 = materials.iter().map(|(_, count)| count).sum();
        if shapes == 0 {
            return 0.0;
        }
        let total: f32 = materials
            .iter()
            .map(|(hash, count)| self.by_hash.get(hash).copied().unwrap_or(0.0) * *count as f32)
            .sum();
        total / shapes as f32
    }

    /// The buoyancy factor of a body: the material mean times the layer and
    /// part multiplier.
    pub fn factor(&self, props: &WaterBodyProps) -> f32 {
        let mean = self.mean(&props.materials);
        mean * buoyancy_multiplier(props.layer, props.part, mean)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use super::*;

    const STONE: u32 = 0xDF02_F237;
    const WOOD: u32 = 0x1DD9_C611;
    const CLOTH: u32 = 0xE4D3_9CA3;

    fn table() -> MaterialBuoyancy {
        MaterialBuoyancy::new([(STONE, -1.0), (WOOD, 0.25), (CLOTH, 0.0)])
    }

    fn props(layer: u8, part: u8, materials: &[(u32, u32)]) -> WaterBodyProps {
        WaterBodyProps {
            layer,
            part,
            materials: materials.to_vec(),
        }
    }

    #[test]
    fn exactly_the_eleven_layers_of_the_game_are_tracked() {
        let tracked: Vec<u8> = (0..=u8::MAX)
            .filter(|layer| is_tracked_layer(*layer))
            .collect();
        assert_eq!(tracked, [4, 5, 6, 8, 10, 12, 13, 14, 29, 32, 33]);
    }

    #[test]
    fn a_body_that_is_not_a_ragdoll_limb_ignores_its_part_id() {
        assert_eq!(buoyancy_multiplier(4, 7, 1.0), 1.0);
        assert_eq!(buoyancy_multiplier(10, 17, 1.0), 1.0);
    }

    #[test]
    fn ragdoll_parts_map_to_body_extremity_or_nothing() {
        let multipliers: Vec<f32> = (0..=23)
            .map(|part| buoyancy_multiplier(8, part, 1.0))
            .collect();
        let body = 1.0;
        let extremity = -0.3;
        let expected = [
            body, // part 0 has no table entry
            body, body, body, body, body, // 1-5
            extremity, extremity, // 6, 7
            body,      // 8
            extremity, extremity, // 9, 10
            body,      // 11
            extremity, extremity, // 12, 13
            body,      // 14
            extremity, extremity, // 15, 16
            0.0, 0.0, 0.0, 0.0, 0.0,       // 17-21
            extremity, // 22
            body,      // 23 is past the table
        ];
        assert_eq!(multipliers, expected);
    }

    #[test]
    fn dead_biped_layers_take_the_part_multiplier_too() {
        for layer in [0x20, 0x21] {
            assert_eq!(buoyancy_multiplier(layer, 6, 1.0), -0.3);
        }
    }

    #[test]
    fn a_limb_whose_material_does_not_float_keeps_the_body_multiplier() {
        assert_eq!(buoyancy_multiplier(8, 6, 0.0), 1.0);
        assert_eq!(buoyancy_multiplier(8, 17, -1.0), 1.0);
    }

    #[test]
    fn the_mean_weights_each_material_by_its_shape_count() {
        let mean = table().mean(&[(STONE, 1), (WOOD, 3)]);
        assert_eq!(mean, (-1.0 + 3.0 * 0.25) / 4.0);
    }

    #[test]
    fn a_hash_without_a_record_contributes_zero_but_still_counts() {
        assert_eq!(table().mean(&[(0xDEAD_BEEF, 1), (WOOD, 1)]), 0.125);
    }

    #[test]
    fn a_body_without_shapes_has_zero_buoyancy() {
        assert_eq!(table().mean(&[]), 0.0);
    }

    #[test]
    fn the_factor_is_the_mean_times_the_layer_part_multiplier() {
        let materials = [(WOOD, 1)];
        assert_eq!(table().factor(&props(4, 0, &materials)), 0.25);
        assert_eq!(table().factor(&props(8, 6, &materials)), 0.25 * -0.3);
        assert_eq!(table().factor(&props(8, 18, &materials)), 0.0);
        assert_eq!(table().factor(&props(8, 6, &[(STONE, 1)])), -1.0);
    }

    #[test]
    fn props_parse_from_the_sidecar_water_object() {
        let json = r#"{"layer":4,"part":0,"materials":[[2086,1],[7,12]]}"#;
        let parsed: WaterBodyProps = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, props(4, 0, &[(2086, 1), (7, 12)]));
    }
}
