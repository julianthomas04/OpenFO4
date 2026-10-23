//! Raw triangle flag bits as authored.

pub const EDGE_0_LINK: u16 = 1 << 0;
pub const EDGE_1_LINK: u16 = 1 << 1;
pub const EDGE_2_LINK: u16 = 1 << 2;
pub const DELETED: u16 = 1 << 3;
pub const NO_LARGE_CREATURES: u16 = 1 << 4;
pub const OVERLAPPING: u16 = 1 << 5;
pub const PREFERRED: u16 = 1 << 6;
pub const WATER: u16 = 1 << 9;
pub const DOOR: u16 = 1 << 10;
pub const FOUND: u16 = 1 << 11;
