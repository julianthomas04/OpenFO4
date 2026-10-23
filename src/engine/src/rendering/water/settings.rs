//! Debug-menu controls of the water stage.

use glam::Vec3;

use crate::bethesda_space::position_from_engine;
use crate::water_world::exterior_cell_of;

/// The cells around the camera cell whose loaded water draws as a full-detail
/// cell surface; further cells draw their LOD quad instead.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CellSurfaceRing {
    center: (i32, i32),
    radius: i32,
}

impl CellSurfaceRing {
    pub fn around(eye: Vec3, radius: i32) -> Self {
        Self {
            center: exterior_cell_of(position_from_engine(eye)),
            radius,
        }
    }

    pub fn contains(self, cell: (i32, i32)) -> bool {
        (cell.0 - self.center.0)
            .abs()
            .max((cell.1 - self.center.1).abs())
            <= self.radius
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct WaterSettings {
    /// Whether the camera's underwater state (surface variant, fogged depth of
    /// field and fog) applies; off renders the camera as if above water.
    pub underwater_effect: bool,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            underwater_effect: true,
        }
    }
}

pub fn water_panel(ui: &mut egui::Ui, settings: &mut WaterSettings) {
    ui.checkbox(&mut settings.underwater_effect, "Underwater camera effect");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bethesda_space::UNITS_TO_ENGINE;

    fn eye_in_cell(x: i32, y: i32) -> Vec3 {
        let size = 4096.0 * UNITS_TO_ENGINE;
        Vec3::new((x as f32 + 0.5) * size, 0.0, -(y as f32 + 0.5) * size)
    }

    #[test]
    fn defaults_keep_the_underwater_effect() {
        assert!(WaterSettings::default().underwater_effect);
    }

    #[test]
    fn the_ring_is_a_square_of_the_given_radius_around_the_camera_cell() {
        let ring = CellSurfaceRing::around(eye_in_cell(-3, 5), 2);
        assert!(ring.contains((-3, 5)));
        assert!(ring.contains((-5, 7)));
        assert!(ring.contains((-1, 3)));
        assert!(!ring.contains((0, 5)));
        assert!(!ring.contains((-3, 8)));
        assert!(!ring.contains((-6, 5)));
    }
}
