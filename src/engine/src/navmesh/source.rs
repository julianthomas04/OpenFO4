use glam::Vec3;

#[derive(Debug, Clone, PartialEq)]
pub struct NavMeshSource {
    pub form_id: u32,
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u16; 3]>,
    pub neighbours: Vec<[i16; 3]>,
    pub flags: Vec<u16>,
    pub edge_links: Vec<EdgeLinkSource>,
    pub door_links: Vec<DoorLinkSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeLinkSource {
    pub triangle: u16,
    pub edge: u8,
    pub link_type: u32,
    pub target_form_id: u32,
    pub target_triangle: u16,
    pub target_edge: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorLinkSource {
    pub triangle: u16,
    pub door_ref_form_id: u32,
}
