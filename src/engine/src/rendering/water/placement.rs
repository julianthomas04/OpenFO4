//! What the water stage draws: placed water meshes the streamer hands over and
//! the distant water quads of the water world.

use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3};

use crate::assets::types::water_shader::{WaterShaderToken, water_flags};
use crate::rendering::frustum::Aabb;
use crate::water_world::{CellPlane, VolumeShapes};

/// A resident water mesh instance (`WorldStreamer::for_each_placed_water`).
#[derive(Debug, Clone, Copy)]
pub struct PlacedWater<'a> {
    pub transform: Mat4,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    pub index_offset: u32,
    pub index_count: u32,
    pub vertex_offset: u32,
    /// WATR form key of the placed activator; `None` when the shape has none.
    pub water_type: Option<&'a str>,
    pub shader: WaterShaderToken,
    /// The collision of the water mesh: the volume the water fills. `None`
    /// when the mesh has no collision sidecar.
    pub volume: Option<&'a Arc<VolumeShapes>>,
}

/// A horizontal water quad in engine space.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterPlane {
    /// World height of the surface (engine units).
    pub height: f32,
    /// Horizontal extent on the engine X and Z axes.
    pub min: Vec2,
    pub max: Vec2,
}

impl From<&CellPlane> for WaterPlane {
    fn from(plane: &CellPlane) -> Self {
        Self {
            height: plane.height,
            min: plane.min,
            max: plane.max,
        }
    }
}

impl WaterPlane {
    /// The unit quad of the stage (`[-0.5, 0.5]` on X and Z at height 0) placed
    /// over this plane.
    pub fn transform(&self) -> Mat4 {
        let center = (self.min + self.max) * 0.5;
        let size = self.max - self.min;
        Mat4::from_translation(Vec3::new(center.x, self.height, center.y))
            * Mat4::from_scale(Vec3::new(size.x, 1.0, size.y))
    }

    pub fn bounds(&self) -> Aabb {
        Aabb {
            min: Vec3::new(self.min.x, self.height, self.min.y),
            max: Vec3::new(self.max.x, self.height, self.max.y),
        }
    }

    /// The shader property of `Water\DefaultProceduralWater.NIF`, whose first
    /// shape every loaded cell's water clones:
    /// `DEPTH | REFLECTIONS | REFRACTIONS` (`0xC4`), shader flag words
    /// `0x80000000` / `0x1`, no uv offset, unit scale (the installer's token
    /// of that NIF).
    pub fn cell_shader() -> WaterShaderToken {
        WaterShaderToken {
            water_flags: water_flags::DEPTH | water_flags::REFLECTIONS | water_flags::REFRACTIONS,
            shader_flags_1: 0x8000_0000,
            shader_flags_2: 1,
            uv_offset: Vec2::ZERO,
            uv_scale: Vec2::ONE,
        }
    }

    /// The shader flags the game gives every LOD water shape: `LOD`, plus
    /// `REFLECTIONS` when `reflections` (`bUseWaterReflections`); `CUBEMAP` only
    /// for `bUseCubeMapReflections`, which the engine does not consult. There is
    /// no depth or refraction flag, so these take the refraction-only path, and
    /// no mesh uv.
    pub fn lod_shader(reflections: bool) -> WaterShaderToken {
        let reflections = if reflections {
            water_flags::REFLECTIONS
        } else {
            0
        };
        WaterShaderToken {
            water_flags: water_flags::LOD | reflections,
            shader_flags_1: 0,
            shader_flags_2: 0,
            uv_offset: Vec2::ZERO,
            uv_scale: Vec2::ONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane() -> WaterPlane {
        WaterPlane {
            height: 45.0,
            min: Vec2::new(-10.0, 20.0),
            max: Vec2::new(30.0, 60.0),
        }
    }

    #[test]
    fn plane_transform_maps_the_unit_quad_onto_the_extent_at_the_height() {
        let transform = plane().transform();
        assert!(
            transform
                .transform_point3(Vec3::new(-0.5, 0.0, -0.5))
                .abs_diff_eq(Vec3::new(-10.0, 45.0, 20.0), 1e-4)
        );
        assert!(
            transform
                .transform_point3(Vec3::new(0.5, 0.0, 0.5))
                .abs_diff_eq(Vec3::new(30.0, 45.0, 60.0), 1e-4)
        );
    }

    #[test]
    fn plane_bounds_are_flat_at_the_height() {
        let bounds = plane().bounds();
        assert!((bounds.min.y - 45.0).abs() < 1e-6);
        assert!((bounds.max.y - 45.0).abs() < 1e-6);
        assert_eq!((bounds.min.x, bounds.max.z), (-10.0, 60.0));
    }

    #[test]
    fn cell_water_is_the_default_procedural_water_property() {
        let shader = WaterPlane::cell_shader();
        assert_eq!(shader.water_flags, 0xC4);
        assert!(!shader.has_flag(water_flags::LOD));
        assert!(!shader.has_flag(water_flags::VERTEX_UV));
        assert!(!shader.has_flag(water_flags::DISPLACEMENT));
    }

    #[test]
    fn lod_water_is_lod_and_reflections_without_depth_or_mesh_uv() {
        let shader = WaterPlane::lod_shader(true);
        assert!(shader.has_flag(water_flags::LOD));
        assert!(shader.has_flag(water_flags::REFLECTIONS));
        assert!(!shader.has_flag(water_flags::DEPTH));
        assert!(!shader.has_flag(water_flags::REFRACTIONS));
        assert!(!shader.has_flag(water_flags::CUBEMAP));
        assert!(!shader.has_flag(water_flags::VERTEX_UV));
    }

    #[test]
    fn lod_water_does_not_reflect_when_the_ini_turns_reflections_off() {
        let shader = WaterPlane::lod_shader(false);
        assert_eq!(shader.water_flags, water_flags::LOD);
    }
}
