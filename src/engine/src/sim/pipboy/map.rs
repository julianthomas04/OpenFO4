//! World map data: the calibrated texture projection and the map markers.

use glam::{Vec2, Vec3};

use super::PipBoyModel;

/// A map marker reference, identified by its form key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MapMarkerId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct MapMarker {
    pub id: MapMarkerId,
    pub name: String,
    /// Raw marker type; selects the `marker/<type>` art clip.
    pub marker_type: u16,
    /// Raw marker flags as authored.
    pub flags: u8,
    /// Engine-space position.
    pub position: Vec3,
    /// The authored Visible flag; the live state is `PipBoyModel::discovered_markers`.
    pub discovered: bool,
    pub can_fast_travel: bool,
}

/// Affine map from an engine-space position to world-map texture UV (`v = 0` is the top row).
/// Calibrated per texture; the database carries no equivalent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldMapProjection {
    pub origin_u: f32,
    pub origin_v: f32,
    pub u_per_engine_x: f32,
    pub v_per_engine_z: f32,
}

impl WorldMapProjection {
    pub const COMMONWEALTH: Self = Self {
        origin_u: 0.536_72,
        origin_v: 0.416_75,
        u_per_engine_x: 3.7324e-5,
        v_per_engine_z: 3.7324e-5,
    };

    pub fn engine_to_uv(&self, position: Vec3) -> Vec2 {
        Vec2::new(
            self.origin_u + self.u_per_engine_x * position.x,
            self.origin_v + self.v_per_engine_z * position.z,
        )
    }
}

/// The map of one worldspace: its texture and markers.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldMap {
    pub worldspace: String,
    /// Asset-root-relative texture key (`textures\interface\pip-boy\worldmap_d.dds`).
    pub texture_key: String,
    pub projection: WorldMapProjection,
    pub markers: Vec<MapMarker>,
}

impl PipBoyModel {
    pub fn is_marker_discovered(&self, marker: &MapMarker) -> bool {
        self.discovered_markers.contains(&marker.id)
    }

    pub fn discover_marker(&mut self, id: MapMarkerId) {
        self.discovered_markers.insert(id);
    }

    pub fn discover_all_markers(&mut self) {
        let ids = self
            .catalog
            .world_map
            .iter()
            .flat_map(|map| &map.markers)
            .map(|marker| marker.id.clone());
        self.discovered_markers.extend(ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bethesda_space::position_to_engine;

    const TEXTURE_PX: f32 = 2048.0;

    fn pixel_of_bethesda_position(x: f32, y: f32) -> Vec2 {
        WorldMapProjection::COMMONWEALTH.engine_to_uv(position_to_engine(x, y, 0.0)) * TEXTURE_PX
    }

    fn assert_within_one_pixel(actual: Vec2, expected: Vec2) {
        assert!(
            (actual - expected).abs().max_element() <= 1.0,
            "{actual} is not within 1 px of {expected}"
        );
    }

    #[test]
    fn sanctuary_lands_on_its_calibrated_pixel() {
        assert_within_one_pixel(
            pixel_of_bethesda_position(-80768.0, 88175.0),
            Vec2::new(482.0, 180.0),
        );
    }

    #[test]
    fn the_castle_lands_on_its_calibrated_pixel() {
        assert_within_one_pixel(
            pixel_of_bethesda_position(48322.0, -46600.0),
            Vec2::new(1469.0, 1210.0),
        );
    }

    fn model_with(markers: Vec<MapMarker>) -> PipBoyModel {
        PipBoyModel::new(super::super::test_support::catalog_with(
            Vec::new(),
            markers,
        ))
    }

    #[test]
    fn a_new_model_starts_with_every_marker_discovered() {
        let model = model_with(vec![
            super::super::test_support::marker("seen", true, true),
            super::super::test_support::marker("hidden", false, true),
        ]);
        let map = model.catalog.world_map.as_ref().unwrap();
        assert!(
            map.markers
                .iter()
                .all(|marker| model.is_marker_discovered(marker))
        );
    }

    #[test]
    fn discovering_markers_is_runtime_state_and_leaves_the_catalog_alone() {
        let mut model = model_with(vec![
            super::super::test_support::marker("a", false, true),
            super::super::test_support::marker("b", false, true),
        ]);
        model.discovered_markers.clear();
        model.discover_marker(MapMarkerId("a".to_owned()));
        let map = model.catalog.world_map.clone().unwrap();
        assert!(model.is_marker_discovered(&map.markers[0]));
        assert!(!model.is_marker_discovered(&map.markers[1]));
        assert!(!map.markers[0].discovered);
        model.discover_all_markers();
        assert!(
            map.markers
                .iter()
                .all(|marker| model.is_marker_discovered(marker))
        );
    }

    #[test]
    fn north_is_up_and_east_is_right() {
        let origin = pixel_of_bethesda_position(0.0, 0.0);
        assert!(pixel_of_bethesda_position(0.0, 4096.0).y < origin.y);
        assert!(pixel_of_bethesda_position(4096.0, 0.0).x > origin.x);
    }
}
