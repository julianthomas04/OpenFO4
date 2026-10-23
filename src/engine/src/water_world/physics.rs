//! The simulation's view of the water world.

use super::SharedWaterWorld;
use crate::sim::water_physics::{WaterQuery, WaterSurface};

/// A [`WaterQuery`] over the shared water world: the surface height at the
/// body and the global current of the water type that holds it.
pub fn physics_query(world: SharedWaterWorld) -> WaterQuery {
    Box::new(move |position| {
        let world = world.read();
        let hit = world.surface_at(position)?;
        Some(WaterSurface {
            height: hit.height,
            current: world.current_of(hit.water_type),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use glam::Vec3;

    use super::*;
    use crate::assets::db::water_placement::{CellWater, CellWaterHeight};
    use crate::water_world::LodWater;
    use crate::water_world::WaterWorld;
    use crate::water_world::height::{WorldspaceChain, WorldspaceWaterRecord};
    use crate::water_world::world::current_of_watr;

    fn world() -> SharedWaterWorld {
        let chain = WorldspaceChain::new(vec![WorldspaceWaterRecord {
            form_key: "ws".to_owned(),
            parent: None,
            parent_flags: 0,
            water_form: Some("ocean".to_owned()),
            lod_water_form: None,
            lod_water_height: None,
            default_water_height: 450.0,
            flags: 0,
        }]);
        let cells = HashMap::from([(
            (0, 0),
            CellWater {
                has_water: true,
                no_lod_water: false,
                height: CellWaterHeight::Unset,
                water_type: None,
                environment_map: None,
                currents: None,
            },
        )]);
        let current = current_of_watr([0.5, 0.0, 0.0], [0.0; 3]).unwrap();
        SharedWaterWorld::new(
            WaterWorld::with_exterior(chain, cells, LodWater::none())
                .with_currents(HashMap::from([("ocean".to_owned(), current)])),
        )
    }

    #[test]
    fn the_query_reports_the_surface_height_and_the_current_of_its_water() {
        let query = physics_query(world());
        // Engine position over cell (0, 0): 450 game units is 45 engine units.
        let surface = query(Vec3::new(10.0, 1.0, -10.0)).unwrap();
        assert!((surface.height - 45.0).abs() < 1e-3);
        assert!(surface.current.unwrap().linear.x > 3.0);
    }

    #[test]
    fn the_query_is_none_outside_any_water() {
        let query = physics_query(world());
        assert!(query(Vec3::new(-1000.0, 1.0, 1000.0)).is_none());
    }
}
