//! Settlements shown on DATA/WORKSHOPS.

use glam::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SettlementStats {
    pub population: u32,
    pub food: u32,
    pub water: u32,
    pub power: u32,
    pub defense: u32,
    pub beds: u32,
    /// Happiness in percent, `0..=100`.
    pub happiness: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settlement {
    pub name: String,
    pub needs_attention: bool,
    pub raider_owned: bool,
    pub stats: SettlementStats,
    /// Engine-space position for SHOW ON MAP.
    pub location: Option<Vec3>,
}
