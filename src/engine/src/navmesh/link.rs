#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkType {
    Portal = 0,
    LedgeUp = 1,
    LedgeDown = 2,
    EnableDisablePortal = 3,
}

impl LinkType {
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Portal),
            1 => Some(Self::LedgeUp),
            2 => Some(Self::LedgeDown),
            3 => Some(Self::EnableDisablePortal),
            _ => None,
        }
    }

    pub const fn mask(self) -> u8 {
        1 << self as u8
    }
}

/// Validated authored link from an edge of one triangle to a triangle of another navmesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeLink {
    pub triangle: u16,
    pub edge: u8,
    pub link_type: LinkType,
    pub target_form_id: u32,
    pub target_triangle: u16,
    pub target_edge: u8,
}
