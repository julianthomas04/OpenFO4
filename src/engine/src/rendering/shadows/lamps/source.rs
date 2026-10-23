use crate::rendering::placed_light::GpuPlacedLight;

/// The LIGH light types that cast shadows. Vanilla decides shadowing by the
/// base light type alone; the REFR `CastsShadows` record flag (0x200) is set on
/// only 15 of 44,425 placed light refs and is ignored.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ShadowType {
    Spot,
    Hemisphere,
    Omni,
}

/// What a placed shadow-type light needs beyond its `GpuPlacedLight` to cast
/// shadows.
#[derive(Debug, Copy, Clone)]
pub struct LampShadowSpec {
    pub shadow_type: ShadowType,
    /// Engine units. Casters nearer to the light than this cast nothing, which
    /// keeps the lamp's own fixture out of its shadow.
    pub near_clip: f32,
    /// XLIG `ShadowDepthBias` as authored. Stored but not applied: its unit is
    /// unknown (modal 2.0, values up to 50 on shadow lights).
    pub depth_bias: Option<f32>,
}

/// A shadow-type light of a resident cell: its index in the cell's light list
/// and its shadow data.
#[derive(Debug, Copy, Clone)]
pub struct ShadowLampSource {
    pub light_index: u32,
    pub spec: LampShadowSpec,
}

/// Identifies a lamp for as long as its cell stays resident: a re-integrated
/// cell gets a new residency, so stale cache entries can never match it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LampKey {
    pub residency: u64,
    pub light_index: u32,
}

/// A resident shadow-type lamp as the selection sees it.
#[derive(Copy, Clone)]
pub struct LampCandidate {
    pub key: LampKey,
    pub light: GpuPlacedLight,
    pub spec: LampShadowSpec,
    /// Index of the light in the placed-light slab.
    pub slab_index: u32,
}
